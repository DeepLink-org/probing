import dataclasses
import os
import re
from typing import Any, Optional, Type, Union

import probing

cache = {}

_DEFAULT_TABLE_CAPACITY_BYTES = 20 * 1024 * 1024


def _env_default_capacity_bytes() -> int:
    raw = os.environ.get("PROBING_TABLE_DEFAULT_MB", "").strip()
    if not raw:
        return _DEFAULT_TABLE_CAPACITY_BYTES
    try:
        mb = int(raw)
    except ValueError:
        return _DEFAULT_TABLE_CAPACITY_BYTES
    if mb <= 0:
        return _DEFAULT_TABLE_CAPACITY_BYTES
    return mb * 1024 * 1024


def _resolve_capacity_bytes(override: Optional[int]) -> int:
    if override is not None:
        return max(override, 4096)
    return _env_default_capacity_bytes()


def camel_to_snake(name):
    """
    Convert CamelCase to snake_case.

    Examples
    --------
    >>> camel_to_snake("CamelCase")
    'camel_case'
    >>> camel_to_snake("SomeVeryLongClassName")
    'some_very_long_class_name'
    """
    s1 = re.sub("(.)([A-Z][a-z]+)", r"\1_\2", name)
    return re.sub("([a-z0-9])([A-Z])", r"\1_\2", s1).lower()


def _table_doc_from_class(cls) -> Optional[str]:
    doc = cls.__doc__
    if not doc:
        return None
    line = doc.strip().splitlines()[0].strip()
    if not line or line.startswith(f"{cls.__name__}("):
        return None
    return line


def _column_docs_from_class(cls) -> dict[str, str]:
    docs: dict[str, str] = {}
    for field in dataclasses.fields(cls):
        meta = field.metadata or {}
        doc = meta.get("doc")
        if doc:
            docs[field.name] = str(doc)
    return docs


def table(
    name_or_class: Optional[Union[str, Type[Any]]] = None,
    *,
    capacity_bytes: Optional[int] = None,
):
    """A decorator that converts a dataclass into a persistable table.

    This decorator adds database-like functionality to dataclasses. When applied to a dataclass,
    it creates or retrieves an ExternalTable with the dataclass name (or provided name) and adds
    methods for data persistence and retrieval operations.

    Table documentation comes from the class ``__doc__`` (first line). Column docs use
    ``field(metadata={"doc": "..."})`` and are registered for SQL ``DESCRIBE``.

    Parameters
    ----------
    name : Optional[str], default=None
        The name of the table to create or access. If None, the decorated class name will be used.
        When provided, the name will be converted to lowercase.
    capacity_bytes : Optional[int], default=None
        Mmap ring byte budget for this table (``discard_threshold``). When omitted, uses
        ``PROBING_TABLE_DEFAULT_MB`` (default 20 MiB per table).

    Returns
    -------
    callable
        A decorator function that adds table functionality to the decorated dataclass.

    Methods Added
    ------------
    append(instance) : classmethod
        Adds a single instance to the table.
    append_many(instances) : classmethod
        Adds multiple instances to the table.
    take(n) : classmethod
        Retrieves n rows from the table.
    drop() : classmethod
        Deletes the table.
    save() : instancemethod
        Saves the current instance to the table.

    Raises
    ------
    TypeError
        If the decorated class is not a dataclass.
    ValueError
        If a table with the same name but different fields already exists.

    Examples
    --------
    >>> from dataclasses import dataclass, field
    >>> @table
    ... @dataclass
    ... class Point:
    ...     \"\"\"Demo points table.\"\"\"
    ...     x: int = field(metadata={"doc": "X coordinate"})
    ...     y: int = field(metadata={"doc": "Y coordinate"})
    >>> Point.append(Point(1, 2))
    >>> Point.take(10)[0][1]
    [1, 2]

    >>> Point(2, 3).save()
    >>> Point.take(10)[1][1]
    [2, 3]
    """

    if isinstance(name_or_class, str):
        cls = None
        name = name_or_class.lower()
    else:
        cls = name_or_class
        name = None

    def decorator(cls):
        if not dataclasses.is_dataclass(cls):
            raise TypeError(f"{cls} is not a dataclass")

        table_name = name or camel_to_snake(cls.__name__)
        fields = [f.name for f in dataclasses.fields(cls)]
        table_doc = _table_doc_from_class(cls)
        column_docs = _column_docs_from_class(cls)
        qualified_name = table_name if "." in table_name else f"python.{table_name}"
        ring_capacity = _resolve_capacity_bytes(capacity_bytes)

        def _register_docs():
            if column_docs or table_doc:
                probing.register_table_docs(
                    qualified_name, table_doc, column_docs or None
                )

        @classmethod
        def init_table(cls):
            if cls in cache:
                return cache[cls]
            try:
                table = probing.ExternalTable.get(table_name)
                if table.names() != fields:
                    raise ValueError(
                        f"Table {table_name} already exists with different fields"
                    )
            except Exception:
                table = probing.ExternalTable(
                    table_name,
                    fields,
                    discard_threshold=ring_capacity,
                    table_doc=table_doc,
                    column_docs=column_docs or None,
                )
            cache[cls] = table
            return table

        # ``dataclasses.astuple`` recurses and *deep-copies* every field; these
        # records are flat scalars, so read them directly. This is on the hot
        # per-row trace path (one call per emitted module/step row).
        def _row(obj):
            return tuple(getattr(obj, f) for f in fields)

        @classmethod
        def append(cls, self):
            table = cls.init_table()
            table.append(_row(self))

        @classmethod
        def append_many(cls, instances):
            table = cls.init_table()
            table.append_many([_row(i) for i in instances])

        @classmethod
        def take(cls, n):
            if cls not in cache:
                return []
            table = cache[cls]
            return table.take(n)

        @classmethod
        def drop(cls):
            if cls in cache:
                del cache[cls]
            probing.ExternalTable.drop(table_name)

        def save(self):
            type(self).append(self)

        setattr(cls, "init_table", init_table)
        setattr(cls, "append", append)
        setattr(cls, "append_many", append_many)
        setattr(cls, "take", take)
        setattr(cls, "drop", drop)
        setattr(cls, "save", save)
        _register_docs()

        return cls

    if cls is not None:
        return decorator(cls)
    return decorator

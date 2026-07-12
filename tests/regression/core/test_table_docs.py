"""Tests for code-first table documentation (@table + register_table_docs)."""

from __future__ import annotations

from dataclasses import dataclass, field

import probing
from probing.core.table import (
    _column_docs_from_class,
    _table_doc_from_class,
)


def test_table_doc_from_class_first_line():
    @dataclass
    class Demo:
        """First line summary.

        More details ignored.
        """

        x: int

    assert _table_doc_from_class(Demo) == "First line summary."


def test_table_doc_from_class_missing():
    @dataclass
    class NoDoc:
        x: int

    assert _table_doc_from_class(NoDoc) is None


def test_column_docs_from_field_metadata():
    @dataclass
    class Demo:
        x: int = field(metadata={"doc": "X coordinate"})
        y: int = field(metadata={"other": "ignored"})

    assert _column_docs_from_class(Demo) == {"x": "X coordinate"}


def test_table_decorator_registers_docs(monkeypatch):
    import importlib

    table_mod = importlib.import_module("probing.core.table")
    table_mod.cache.clear()
    table_name = f"decorated_doc_{id(object())}"
    captured: dict = {}

    def capture_register(qualified, table_doc, column_docs):
        captured["qualified"] = qualified
        captured["table_doc"] = table_doc
        captured["column_docs"] = column_docs or {}
        return probing._core.register_table_docs(qualified, table_doc, column_docs)

    monkeypatch.setattr(probing, "register_table_docs", capture_register)

    @table_mod.table(table_name)
    @dataclass
    class DecoratedMetrics:
        """Decorated metrics table."""

        latency_ms: float = field(metadata={"doc": "latency milliseconds"})

    assert captured["qualified"] == f"python.{table_name}"
    assert captured["table_doc"] == "Decorated metrics table."
    assert captured["column_docs"]["latency_ms"] == "latency milliseconds"

    DecoratedMetrics.drop()
    table_mod.cache.clear()


def test_table_decorator_defers_mmap_until_append(monkeypatch):
    import importlib

    table_mod = importlib.import_module("probing.core.table")
    table_mod.cache.clear()
    table_name = f"lazy_mmap_{id(object())}"

    class FakeExternalTable:
        created: list[str] = []

        @staticmethod
        def get(_name):
            raise ValueError("missing")

        @staticmethod
        def drop(_name):
            return None

        def __init__(self, name, columns, **kwargs):
            FakeExternalTable.created.append(name)
            self._columns = columns

        def names(self):
            return list(self._columns)

        def append(self, _row):
            return None

        def take(self, _n):
            return []

    FakeExternalTable.created = []
    monkeypatch.setattr(probing, "ExternalTable", FakeExternalTable)
    monkeypatch.setattr(probing, "register_table_docs", lambda *a, **k: None)

    @table_mod.table(table_name)
    @dataclass
    class LazyMetrics:
        value: int = 0

    assert FakeExternalTable.created == []
    LazyMetrics.append(LazyMetrics(1))
    assert FakeExternalTable.created == [table_name]
    LazyMetrics.drop()
    table_mod.cache.clear()


def test_table_capacity_bytes_override(monkeypatch):
    import importlib

    table_mod = importlib.import_module("probing.core.table")
    table_mod.cache.clear()
    table_name = f"cap_table_{id(object())}"
    captured: dict = {}

    class FakeExternalTable:
        @staticmethod
        def get(_name):
            raise ValueError("missing")

        @staticmethod
        def drop(_name):
            return None

        def __init__(self, name, columns, **kwargs):
            captured.update(kwargs)
            self._columns = columns

        def names(self):
            return list(self._columns)

        def append(self, _row):
            return None

    monkeypatch.setattr(probing, "ExternalTable", FakeExternalTable)
    monkeypatch.setattr(probing, "register_table_docs", lambda *a, **k: None)

    @table_mod.table(table_name, capacity_bytes=4 * 1024 * 1024)
    @dataclass
    class SizedMetrics:
        value: int = 0

    SizedMetrics.append(SizedMetrics(1))
    assert captured.get("discard_threshold") == 4 * 1024 * 1024
    SizedMetrics.drop()
    table_mod.cache.clear()


def test_env_default_capacity_bytes(monkeypatch):
    import importlib

    table_mod = importlib.import_module("probing.core.table")
    monkeypatch.setenv("PROBING_TABLE_DEFAULT_MB", "8")
    assert table_mod._resolve_capacity_bytes(None) == 8 * 1024 * 1024
    monkeypatch.delenv("PROBING_TABLE_DEFAULT_MB", raising=False)
    assert table_mod._resolve_capacity_bytes(None) == 20 * 1024 * 1024


def test_builtin_hccl_docs_in_engine_catalog():
    """HCCL code-first docs are baked into the semantic catalog at engine build."""
    df = probing.query(
        "SELECT description FROM probe.probing.column_docs "
        "WHERE table_schema = 'hccl' AND table_name = 'tasks' "
        "AND column_name = 'task_name'"
    )
    assert len(df) == 1
    assert "Memcpy" in str(df["description"].iloc[0])

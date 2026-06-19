"""
Probing Core Engine Module.

This module provides the core functionality for executing SQL queries and
loading Rust extensions in the Probing library. It serves as the primary
interface between Python code and the underlying Rust implementation.
"""

from __future__ import annotations

import json
from typing import Any


def _col_values(column: Any) -> list[Any]:
    if isinstance(column, dict):
        return next(iter(column.values()))
    return column


def _dataframe_from_proto(data: dict[str, Any]):
    import pandas as pd

    frame = {name: _col_values(col) for name, col in zip(data["names"], data["cols"])}
    return pd.DataFrame(frame)


def query(sql: str) -> "DataFrame":  # noqa: F821
    """Execute a SQL query and return the result as a pandas DataFrame."""
    from probing import _core

    ret = _core.query_json(sql)
    if not ret or ret == "null":
        try:
            import pandas as pd

            return pd.DataFrame()
        except ImportError:
            return None  # type: ignore[return-value]

    try:
        import pandas as pd

        data = json.loads(ret)
        if data is None:
            return pd.DataFrame()
        if isinstance(data, dict) and "names" in data and "cols" in data:
            return _dataframe_from_proto(data)
        raise RuntimeError(f"unexpected query_json response: {ret[:500]}")
    except ImportError:
        return ret


def load_extension(statement: str):
    """Load a Rust extension into the probing library."""
    import importlib
    import sys

    parts = statement.split(".")
    if parts[0] not in sys.modules:
        importlib.import_module(parts[0])
    root = sys.modules[parts[0]]
    module = f"{parts[0]}"
    for part in parts[1:]:
        if not hasattr(root, part):
            importlib.import_module(module + "." + part)
        module = f"{module}.{part}"

    return eval(
        statement,
        None,
        {
            parts[0]: sys.modules[parts[0]],
        },
    )

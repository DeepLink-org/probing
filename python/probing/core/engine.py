"""
Probing Core Engine Module.

This module provides the core functionality for executing SQL queries and
loading Rust extensions in the Probing library. It serves as the primary
interface between Python code and the underlying Rust implementation.
"""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from typing import Any


@dataclass(frozen=True)
class QueryQuality:
    nodes_succeeded: int = 0
    nodes_failed: list[str] = field(default_factory=list)
    peer_batches_dropped: int = 0
    partial: bool = False


@dataclass(frozen=True)
class QueryOutcome:
    data: Any
    quality: QueryQuality


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


def query_outcome(sql: str) -> QueryOutcome:
    """Execute SQL and preserve distributed completeness metadata."""
    from probing import _core

    payload = json.loads(_core.query_outcome_json(sql))
    raw_quality = payload.get("quality") or {}
    quality = QueryQuality(
        nodes_succeeded=int(raw_quality.get("nodes_succeeded", 0)),
        nodes_failed=list(raw_quality.get("nodes_failed") or []),
        peer_batches_dropped=int(raw_quality.get("peer_batches_dropped", 0)),
        partial=bool(raw_quality.get("partial", False)),
    )
    raw_data = payload.get("data")
    data = None if raw_data is None else _dataframe_from_proto(raw_data)
    return QueryOutcome(data=data, quality=quality)


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

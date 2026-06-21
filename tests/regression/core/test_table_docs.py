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


def test_builtin_hccl_docs_in_engine_catalog():
    """HCCL code-first docs are baked into the semantic catalog at engine build."""
    df = probing.query(
        "SELECT description FROM probe.probing.column_docs "
        "WHERE table_schema = 'hccl' AND table_name = 'tasks' "
        "AND column_name = 'task_name'"
    )
    assert len(df) == 1
    assert "Memcpy" in str(df["description"].iloc[0])

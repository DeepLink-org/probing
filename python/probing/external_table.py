"""Python wrapper for ``probing._core.ExternalTable`` with main-thread dispatch."""

from __future__ import annotations

from typing import Any

from probing import _core
from probing._native import call_native


class ExternalTable:
    """Proxy around the Rust ExternalTable that avoids macOS SIGSEGV after PyArrow."""

    __slots__ = ("_inner",)

    def __init__(
        self,
        name: str,
        columns: list[str],
        chunk_size: int = 10000,
        discard_threshold: int = 20_000_000,
        discard_strategy: str = "BaseMemorySize",
    ) -> None:
        self._inner = call_native(
            _core.ExternalTable,
            name,
            columns,
            chunk_size,
            discard_threshold,
            discard_strategy,
        )

    @classmethod
    def get(cls, name: str) -> ExternalTable:
        obj = cls.__new__(cls)
        obj._inner = call_native(_core.ExternalTable.get, name)
        return obj

    @classmethod
    def get_or_create(
        cls,
        name: str,
        columns: list[str],
        chunk_size: int = 10000,
        discard_threshold: int = 20_000_000,
        discard_strategy: str = "BaseMemorySize",
    ) -> ExternalTable:
        obj = cls.__new__(cls)
        obj._inner = call_native(
            _core.ExternalTable.get_or_create,
            name,
            columns,
            chunk_size,
            discard_threshold,
            discard_strategy,
        )
        return obj

    @classmethod
    def drop(cls, name: str) -> None:
        call_native(_core.ExternalTable.drop, name)

    def names(self) -> list[str]:
        return call_native(self._inner.names)

    def append(self, values: list[Any]) -> None:
        call_native(self._inner.append, values)

    def append_ts(self, t: int, values: list[Any]) -> None:
        call_native(self._inner.append_ts, t, values)

    def append_many(self, rows: list[list[Any]]) -> None:
        call_native(self._inner.append_many, rows)

    def take(self, limit: int | None = None) -> list[tuple[Any, list[Any]]]:
        return call_native(self._inner.take, limit)

"""Fixtures for extension tests that should run without a built Rust wheel."""

from __future__ import annotations

import sys
import types
from unittest.mock import MagicMock


class _FakeExternalTable:
    _tables: dict[str, "_FakeExternalTable"] = {}

    def __init__(self, name: str, fields: list[str]):
        self.name = name
        self._fields = list(fields)
        self._rows: list[tuple] = []

    @classmethod
    def get(cls, name: str) -> "_FakeExternalTable":
        if name not in cls._tables:
            raise KeyError(name)
        return cls._tables[name]

    @classmethod
    def get_or_create(cls, name: str, fields: list[str]) -> "_FakeExternalTable":
        table = cls._tables.get(name)
        if table is None:
            table = cls(name, fields)
            cls._tables[name] = table
        return table

    @classmethod
    def drop(cls, name: str) -> None:
        cls._tables.pop(name, None)

    def append(self, row: tuple) -> None:
        self._rows.append(row)

    def append_many(self, rows: list[tuple]) -> None:
        self._rows.extend(rows)

    def take(self, n: int) -> list[tuple]:
        return self._rows[:n]

    def names(self) -> list[str]:
        return list(self._fields)


def _install_probing_core_stub() -> None:
    if "probing._core" in sys.modules:
        return

    fake_core = types.ModuleType("probing._core")
    fake_core.ExternalTable = _FakeExternalTable
    fake_core.TCPStore = MagicMock
    fake_core.cli_main = MagicMock()
    fake_core.enable_tracer = MagicMock()
    fake_core.disable_tracer = MagicMock()
    fake_core.is_enabled = MagicMock(return_value=True)
    fake_core.config_get = MagicMock(return_value=None)
    fake_core.config_get_str = MagicMock(return_value=None)
    fake_core.config_set = MagicMock()
    fake_core.config_contains_key = MagicMock(return_value=False)
    fake_core.config_remove = MagicMock()
    fake_core.config_keys = MagicMock(return_value=[])
    fake_core.config_clear = MagicMock()
    fake_core.config_len = MagicMock(return_value=0)
    fake_core.config_is_empty = MagicMock(return_value=True)
    fake_core._get_python_stacks = MagicMock(return_value=[])
    fake_core._get_python_frames = MagicMock(return_value=[])
    sys.modules["probing._core"] = fake_core


_install_probing_core_stub()

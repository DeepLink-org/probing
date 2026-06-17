"""Thin export of ``probing._core.ExternalTable`` (Rust handles thread safety)."""

from probing import _core

ExternalTable = _core.ExternalTable

"""Fixtures for unit tests (no probing engine / ``_core`` required)."""

from __future__ import annotations

import os

import pytest

# Unit tests must not start the in-process probing server.
os.environ["PROBING"] = "0"
os.environ.pop("PROBING_ORIGINAL", None)


def pytest_collection_modifyitems(items: list[pytest.Item]) -> None:
    for item in items:
        if "tests/unit/" in str(item.fspath):
            item.add_marker(pytest.mark.unit)

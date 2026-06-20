"""Fixtures for regression / integration Python tests (needs ``probing._core``)."""

from __future__ import annotations

import os
import time

import pytest

os.environ.setdefault("PROBING", "1")


@pytest.fixture(scope="session", autouse=True)
def _wait_for_probing_engine():
    """Brief pause so the in-process probing server can finish starting."""
    enabled = os.environ.get("PROBING_ORIGINAL") or os.environ.get("PROBING")
    if enabled and str(enabled).lower() not in ("0", "false", "no", ""):
        time.sleep(1.0)
    yield


_COLLECTIVE_CONFIG_KEYS: tuple[str, ...] = (
    "probing.torch.collective.enable",
    "probing.torch.collective.mode",
    "probing.torch.collective.trace_event",
    "probing.torch.collective.verbose",
    "probing.torch.collective.sync",
    "probing.torch.collective.trace_file",
    "probing.torch.collective.resolve_ranks",
)


@pytest.fixture(autouse=True)
def _reset_collective_config(monkeypatch):
    """Reset collective-related config and rank env between tests."""
    import probing

    monkeypatch.delenv("WORLD_SIZE", raising=False)
    monkeypatch.delenv("RANK", raising=False)
    for key in _COLLECTIVE_CONFIG_KEYS:
        try:
            probing.config.remove(key)
        except Exception:
            pass
    yield
    for key in _COLLECTIVE_CONFIG_KEYS:
        try:
            probing.config.remove(key)
        except Exception:
            pass


def pytest_collection_modifyitems(items: list[pytest.Item]) -> None:
    for item in items:
        path = str(item.fspath)
        if "tests/regression/" in path and "/rust/" not in path:
            item.add_marker(pytest.mark.regression)

"""Fixtures for regression / integration Python tests (needs ``probing._core``)."""

from __future__ import annotations

import os
import time

import pytest

os.environ.setdefault("PROBING", "1")


@pytest.fixture(scope="session", autouse=True)
def _wait_for_probing_engine():
    """Wait until the in-process probing engine accepts SQL (before IPython threads)."""
    enabled = os.environ.get("PROBING_ORIGINAL") or os.environ.get("PROBING")
    if enabled and str(enabled).lower() not in ("0", "false", "no", ""):
        import sys

        import probing

        # VM eval-frame hook conflicts with pytest-cov on Linux (SIGABRT in query path).
        if sys.gettrace() is not None or os.environ.get("PROBING_VM_TRACER") == "0":
            try:
                probing.disable_tracer()
            except Exception:
                pass

        deadline = time.monotonic() + 30.0
        last_error: Exception | None = None
        while time.monotonic() < deadline:
            try:
                df = probing.query("SELECT 1 AS ok")
                if len(df) == 1 and df["ok"].tolist() == [1]:
                    break
            except Exception as exc:
                last_error = exc
            time.sleep(0.2)
        else:
            msg = "probing engine did not become ready within 30s"
            if last_error is not None:
                msg = f"{msg}: {last_error}"
            pytest.fail(msg)
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

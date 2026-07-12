"""Profiling regression tests — keep deferred drain synchronous by default."""

from __future__ import annotations

import pytest

from probing.profiling.deferred_drain import reset_deferred_drain_worker_for_tests


@pytest.fixture(autouse=True)
def _sync_torch_deferred_drain(monkeypatch):
    """Regression tests expect immediate ``_drain_deferred`` side effects."""
    monkeypatch.setenv("PROBING_TORCH_DEFER_ASYNC", "0")
    yield
    reset_deferred_drain_worker_for_tests()


@pytest.fixture(autouse=True)
def _reset_torch_profiler_state():
    from probing.profiling.torch_profiler.controller import (
        reset_torch_profiler_for_tests,
    )

    reset_torch_profiler_for_tests()
    yield
    reset_torch_profiler_for_tests()

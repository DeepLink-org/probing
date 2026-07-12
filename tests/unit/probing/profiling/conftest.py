"""Unit-test isolation for torch_profiler."""

from __future__ import annotations

import pytest


@pytest.fixture(autouse=True)
def _reset_torch_profiler_state():
    from probing.profiling.torch_profiler.controller import (
        reset_torch_profiler_for_tests,
    )

    reset_torch_profiler_for_tests()
    yield
    reset_torch_profiler_for_tests()

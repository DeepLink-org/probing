"""Unit tests for the public :mod:`probing.timing` API.

These cover the ``@timing`` decorator and ``TimingContext`` bookkeeping. The
underlying ``AcceleratorTimer`` (which needs a GPU) is replaced with a fake so
the context/contextvar behaviour can be tested on any machine.
"""

from __future__ import annotations

import pytest

import probing.timing as timing_mod
from probing.timing import TimingRecord, timing, timing_context


class _FakeTimer:
    """Runs the workload on the host and returns a fixed elapsed time."""

    elapsed_ms = 1.5
    method = "cuda_event_wait_value32_ffi"

    def __init__(self, torch, device):
        self.torch = torch
        self.device = device

    def run(self, fn, *args, **kwargs):
        value = fn(*args, **kwargs)
        return TimingRecord(self.elapsed_ms, self.method, value)


@pytest.fixture(autouse=True)
def _fake_timer(monkeypatch):
    """Swap in the host-side fake timer and guarantee a clean contextvar."""
    monkeypatch.setattr(timing_mod, "AcceleratorTimer", _FakeTimer)
    assert timing_mod._CURRENT_CONTEXT.get() is None
    yield
    assert timing_mod._CURRENT_CONTEXT.get() is None


def test_timing_runs_untouched_without_active_context():
    calls = []

    @timing
    def workload(x):
        calls.append(x)
        return x * 2

    assert workload(21) == 42
    assert calls == [21]


def test_all_exports_public_api():
    assert set(timing_mod.__all__) == {
        "timing",
        "timing_context",
        "TimingContext",
        "TimingRecord",
        "AcceleratorTimer",
    }


def test_context_records_decorated_workload():
    @timing
    def gemm():
        return "result"

    with timing_context(torch=object(), device=0) as ctx:
        returned = gemm()

    assert returned == "result"
    assert ctx["gemm"] == _FakeTimer.elapsed_ms
    assert ctx.values["gemm"] == "result"
    assert ctx.methods["gemm"] == "cuda_event_wait_value32_ffi"
    assert ctx.records["gemm"].elapsed_ms == _FakeTimer.elapsed_ms


def test_decorator_forwards_args_and_kwargs():
    @timing
    def add(a, b, *, c):
        return a + b + c

    with timing_context(torch=object()) as ctx:
        total = add(1, 2, c=3)

    assert total == 6
    assert ctx.values["add"] == 6


def test_record_uses_function_name_as_key():
    def raw():
        return 7

    with timing_context(torch=object()) as ctx:
        value = ctx.record("custom_label", raw)

    assert value == 7
    assert "custom_label" in ctx.records
    assert ctx["custom_label"] == _FakeTimer.elapsed_ms


def test_context_restores_previous_on_exit():
    @timing
    def work():
        return timing_mod._CURRENT_CONTEXT.get()

    with timing_context(torch=object()) as outer:
        assert timing_mod._CURRENT_CONTEXT.get() is outer
        with timing_context(torch=object()) as inner:
            assert timing_mod._CURRENT_CONTEXT.get() is inner
            work()
        # Inner block exited: the outer context must be active again.
        assert timing_mod._CURRENT_CONTEXT.get() is outer
        assert "work" in inner.records
        assert "work" not in outer.records

    assert timing_mod._CURRENT_CONTEXT.get() is None


def test_workload_exception_propagates_through_context():
    @timing
    def boom():
        raise RuntimeError("kernel launch failed")

    with timing_context(torch=object()) as ctx:
        with pytest.raises(RuntimeError, match="kernel launch failed"):
            boom()

    assert ctx.records == {}

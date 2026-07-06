"""Unit tests for :mod:`probing.timing.timer`.

``AcceleratorTimer`` orchestrates CUDA events and the stream-value gate. These
tests replace the gate and event helpers with fakes so the ordering guarantees
(gate reset/block -> start -> workload -> end -> release -> sync) can be checked
without a GPU.
"""

from __future__ import annotations

import pytest

from probing.timing import timer as timer_mod
from probing.timing.timer import METHOD, AcceleratorTimer, TimingRecord


class _FakeGate:
    def __init__(self, log):
        self._log = log

    def reset(self, stream):
        self._log.append(("reset", stream))

    def block(self, stream):
        self._log.append(("block", stream))

    def release(self):
        self._log.append(("release", None))


class _FakeEvent:
    def __init__(self, name, log):
        self._name = name
        self._log = log

    def synchronize(self):
        self._log.append(("sync", self._name))

    def elapsed_time(self, other):
        # Log the event the method is invoked on so the ordering test can assert
        # the real code computes ``start.elapsed_time(end)`` (not the reverse).
        self._log.append(("elapsed_time", self._name))
        return 2.5


@pytest.fixture
def _timer_env(monkeypatch):
    """Build an ``AcceleratorTimer`` whose gate and events are fakes."""
    log: list = []
    gate = _FakeGate(log)
    start = _FakeEvent("start", log)
    end = _FakeEvent("end", log)
    stream = "STREAM"

    monkeypatch.setattr(timer_mod, "acquire_gate", lambda torch, device: gate)
    monkeypatch.setattr(
        timer_mod, "event_pair", lambda torch, device: (start, end, stream)
    )
    monkeypatch.setattr(
        timer_mod,
        "record_event",
        lambda event, stream: log.append(("record", event._name, stream)),
    )

    timer = AcceleratorTimer(torch=object(), device=0)
    return timer, log, stream


def test_timing_record_is_frozen():
    record = TimingRecord(1.0, "m", "v")
    assert (record.elapsed_ms, record.method, record.value) == (1.0, "m", "v")
    with pytest.raises(Exception):
        record.elapsed_ms = 2.0  # type: ignore[misc]


def test_method_property_reports_ffi_method(_timer_env):
    timer, _log, _stream = _timer_env
    assert timer.method == METHOD == "cuda_event_wait_value32_ffi"


def test_run_returns_workload_value_and_elapsed(_timer_env):
    timer, _log, _stream = _timer_env

    record = timer.run(lambda a, b: a + b, 2, b=3)

    assert isinstance(record, TimingRecord)
    assert record.value == 5
    assert record.elapsed_ms == 2.5
    assert record.method == METHOD


def test_run_orders_gate_and_events(_timer_env):
    timer, log, stream = _timer_env

    timer.run(lambda: "ok")

    assert log == [
        ("reset", stream),
        ("block", stream),
        ("record", "start", stream),
        ("record", "end", stream),
        ("release", None),
        ("sync", "end"),
        ("elapsed_time", "start"),
    ]


def test_run_releases_gate_when_workload_raises(_timer_env):
    timer, log, stream = _timer_env

    def boom():
        raise ValueError("workload failed")

    with pytest.raises(ValueError, match="workload failed"):
        timer.run(boom)

    # The end event is never recorded, but the gate must still be released so
    # the compute stream does not stay blocked behind the device-side flag.
    assert log == [
        ("reset", stream),
        ("block", stream),
        ("record", "start", stream),
        ("release", None),
    ]

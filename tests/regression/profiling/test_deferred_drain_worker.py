"""Background deferred-drain worker tests."""

from __future__ import annotations

import threading

import pytest

from probing.profiling.deferred_drain import (
    DeferredDrainWorker,
    reset_deferred_drain_worker_for_tests,
)
from probing.profiling.torch_probe import DelayedRecord, TorchProbe


class _FakeRecord:
    def __init__(self) -> None:
        self.saved = False
        self.save_thread: int | None = None

    def save(self) -> None:
        self.saved = True
        self.save_thread = threading.get_ident()


@pytest.fixture(autouse=True)
def _enable_async_drain(monkeypatch):
    monkeypatch.setenv("PROBING_TORCH_DEFER_ASYNC", "1")
    reset_deferred_drain_worker_for_tests()
    yield
    reset_deferred_drain_worker_for_tests()


def test_worker_persists_enqueued_records(monkeypatch):
    worker = DeferredDrainWorker(maxsize=16)
    monkeypatch.setattr(
        "probing.profiling.deferred_drain.get_deferred_drain_worker",
        lambda: worker,
    )

    record = _FakeRecord()
    assert worker.submit(DelayedRecord(record, None))
    worker.flush(timeout=2.0)
    assert record.saved


def test_save_runs_off_main_thread(monkeypatch):
    worker = DeferredDrainWorker(maxsize=16)
    monkeypatch.setattr(
        "probing.profiling.deferred_drain.get_deferred_drain_worker",
        lambda: worker,
    )
    main_tid = threading.get_ident()
    record = _FakeRecord()
    assert worker.submit(DelayedRecord(record, None))
    worker.flush(timeout=2.0)
    assert record.saved
    assert record.save_thread is not None
    assert record.save_thread != main_tid


def test_queue_full_falls_back_to_sync_save(monkeypatch):
    from probing.profiling.deferred_drain import _DrainTask

    worker = DeferredDrainWorker(maxsize=1)
    monkeypatch.setattr(
        "probing.profiling.deferred_drain.get_deferred_drain_worker",
        lambda: worker,
    )
    monkeypatch.setattr(worker, "_ensure_started", lambda: None)

    blocker = _FakeRecord()
    worker._queue.put_nowait(
        _DrainTask(record=DelayedRecord(blocker, None), force=False)
    )

    overflow = _FakeRecord()
    tracer = TorchProbe()
    tracer._persist_deferred_records([(DelayedRecord(overflow, None), False)])
    assert overflow.saved
    assert overflow.save_thread == threading.get_ident()
    assert not blocker.saved


def test_drain_enqueues_without_blocking_hook(monkeypatch):
    worker = DeferredDrainWorker(maxsize=16)
    monkeypatch.setattr(
        "probing.profiling.deferred_drain.get_deferred_drain_worker",
        lambda: worker,
    )

    slow = _FakeRecord()
    gate = threading.Event()

    def slow_save() -> None:
        gate.wait(timeout=2.0)
        slow.saved = True

    slow.save = slow_save  # type: ignore[method-assign]

    tracer = TorchProbe()
    tracer._deferred = []
    ready_dr = DelayedRecord(slow, None)
    tracer._persist_deferred_records([(ready_dr, False)])

    assert worker.pending() >= 1
    assert not slow.saved
    gate.set()
    worker.flush(timeout=2.0)
    assert slow.saved

"""Background persistence for deferred TorchProbe GPU trace records."""

from __future__ import annotations

import atexit
import logging
import os
import queue
import threading
from dataclasses import dataclass
from typing import TYPE_CHECKING, Optional

from probing.util.env import parse_bool_flag

if TYPE_CHECKING:
    from probing.profiling.torch_probe import DelayedRecord

logger = logging.getLogger(__name__)

_ENV_ASYNC = "PROBING_TORCH_DEFER_ASYNC"
_ENV_QUEUE_SIZE = "PROBING_TORCH_DEFER_QUEUE_SIZE"
_DEFAULT_QUEUE_SIZE = 4096


def deferred_drain_async_enabled() -> bool:
    """Return whether deferred saves run on a background worker (default: on)."""
    flag = parse_bool_flag(os.environ.get(_ENV_ASYNC))
    if flag is None:
        return True
    return flag


def _queue_maxsize() -> int:
    raw = os.environ.get(_ENV_QUEUE_SIZE)
    if raw is None:
        return _DEFAULT_QUEUE_SIZE
    try:
        size = int(raw)
    except (TypeError, ValueError):
        return _DEFAULT_QUEUE_SIZE
    return max(1, size)


@dataclass(frozen=True)
class _DrainTask:
    record: DelayedRecord
    force: bool


class DeferredDrainWorker:
    """Bounded queue + daemon thread for ``DelayedRecord.save()``."""

    def __init__(self, *, maxsize: int) -> None:
        self._maxsize = maxsize
        self._queue: queue.Queue[Optional[_DrainTask]] = queue.Queue(maxsize=maxsize)
        self._thread: Optional[threading.Thread] = None
        self._started = False
        self._atexit_registered = False
        self._lock = threading.Lock()

    @property
    def enabled(self) -> bool:
        return deferred_drain_async_enabled()

    def pending(self) -> int:
        return self._queue.qsize()

    def _ensure_started(self) -> None:
        with self._lock:
            if self._started:
                return
            self._thread = threading.Thread(
                target=self._run,
                name="probing-torch-deferred-drain",
                daemon=True,
            )
            self._thread.start()
            self._started = True
            if not self._atexit_registered:
                atexit.register(shutdown_deferred_drain)
                self._atexit_registered = True

    def submit(self, dr: DelayedRecord, *, force: bool = False) -> bool:
        """Enqueue a deferred save. Returns False when async is off or the queue is full."""
        if not self.enabled:
            return False
        self._ensure_started()
        try:
            self._queue.put_nowait(_DrainTask(record=dr, force=force))
            return True
        except queue.Full:
            logger.debug(
                "deferred drain queue full (%s slots); falling back to sync save",
                self._maxsize,
            )
            return False

    def _run(self) -> None:
        while True:
            task = self._queue.get()
            try:
                if task is None:
                    return
                _process_task(task)
            except Exception:
                logger.debug("deferred drain task failed", exc_info=True)
            finally:
                self._queue.task_done()

    def flush(self, timeout: Optional[float] = 10.0) -> None:
        """Block until all queued saves complete."""
        if not self._started:
            return
        if timeout is None:
            self._queue.join()
            return
        done = threading.Event()

        def _waiter() -> None:
            self._queue.join()
            done.set()

        threading.Thread(
            target=_waiter, name="probing-deferred-drain-flush", daemon=True
        ).start()
        if not done.wait(timeout=timeout):
            logger.warning(
                "deferred drain flush timed out after %.1fs (%s tasks still queued)",
                timeout,
                self.pending(),
            )

    def shutdown(self, timeout: float = 5.0) -> None:
        with self._lock:
            if not self._started or self._thread is None:
                return
            thread = self._thread
        self.flush(timeout=timeout)
        try:
            self._queue.put_nowait(None)
        except queue.Full:
            logger.warning(
                "deferred drain shutdown: queue full, worker may not exit cleanly"
            )
        thread.join(timeout=timeout)


def _process_task(task: _DrainTask) -> None:
    from probing.profiling.torch_probe import TorchProbe

    if task.force:
        TorchProbe._force_event_complete(task.record)
    task.record.save()


_worker: Optional[DeferredDrainWorker] = None
_worker_lock = threading.Lock()


def get_deferred_drain_worker() -> DeferredDrainWorker:
    global _worker
    with _worker_lock:
        if _worker is None:
            _worker = DeferredDrainWorker(maxsize=_queue_maxsize())
        return _worker


def flush_deferred_drain(timeout: float = 10.0) -> None:
    """Wait for the background drain queue to empty (tests / graceful shutdown)."""
    get_deferred_drain_worker().flush(timeout=timeout)


def shutdown_deferred_drain(timeout: float = 5.0) -> None:
    """Flush and stop the background drain worker."""
    global _worker
    with _worker_lock:
        worker = _worker
        _worker = None
    if worker is not None:
        worker.shutdown(timeout=timeout)


def reset_deferred_drain_worker_for_tests() -> None:
    """Tear down the singleton worker between tests."""
    shutdown_deferred_drain(timeout=1.0)

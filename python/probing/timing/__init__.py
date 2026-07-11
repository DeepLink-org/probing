"""
Accelerator Timing (experimental)

Spec
----
This package measures how long CUDA workloads take using stream-value-gated
CUDA events (the ``cuda_event_wait_value32_ffi`` method), which exclude host
enqueue latency from the measurement.

Not wired into production training paths yet — used by unit/regression tests.

Usage:
    from probing.timing import timing, timing_context

    @timing
    def gemm():
        return a @ b

    with timing_context(device=0) as ctx:
        gemm()
    elapsed_ms = ctx["gemm"]
    method = ctx.methods["gemm"]

A ``@timing`` workload is timed only while a ``timing_context`` is active on the
current thread; outside a context it runs untouched. Timing requires a CUDA
device that supports driver stream memory operations; there is no fallback
timer.

Public Interfaces:
- `timing`: decorate a workload so the active context records its elapsed time.
- `timing_context`: open a context that collects ``@timing`` measurements.
- `TimingContext`: the context object (records / values / methods).
- `TimingRecord`: a single measurement (elapsed_ms / method / value).
- `AcceleratorTimer`: the underlying timer for standalone use.
"""

from __future__ import annotations

import contextvars
import functools

from probing.timing.backend import _torch
from probing.timing.timer import AcceleratorTimer, TimingRecord

_CURRENT_CONTEXT = contextvars.ContextVar("probing_timing_context", default=None)


class TimingContext:
    """Collect timings from ``@timing`` workloads run within a ``with`` block.

    Indexing the context (``ctx["workload"]``) returns the elapsed milliseconds;
    ``records`` / ``values`` / ``methods`` expose the full record, the workload
    return value, and the timing method keyed by workload name.
    """

    def __init__(self, torch=None, device: int = 0):
        self._torch = torch or _torch()
        self._device = device
        self._timer = AcceleratorTimer(self._torch, device)
        self._token = None
        self.records: dict[str, TimingRecord] = {}
        self.values: dict[str, object] = {}
        self.methods: dict[str, str] = {}

    def __enter__(self) -> "TimingContext":
        self._token = _CURRENT_CONTEXT.set(self)
        return self

    def __exit__(self, exc_type, exc, tb) -> bool:
        _CURRENT_CONTEXT.reset(self._token)
        return False

    def __getitem__(self, workload: str) -> float:
        return self.records[workload].elapsed_ms

    def record(self, workload: str, fn, *args, **kwargs):
        result = self._timer.run(fn, *args, **kwargs)
        self.records[workload] = result
        self.values[workload] = result.value
        self.methods[workload] = result.method
        return result.value


def timing(fn):
    """Decorate a workload so an active ``timing_context`` records its time.

    When no context is active on the current thread, the wrapped function runs
    exactly as if it were undecorated.
    """

    @functools.wraps(fn)
    def wrapper(*args, **kwargs):
        ctx = _CURRENT_CONTEXT.get()
        if ctx is None:
            return fn(*args, **kwargs)
        return ctx.record(fn.__name__, fn, *args, **kwargs)

    return wrapper


def timing_context(torch=None, device: int = 0) -> TimingContext:
    """Open a context that records ``@timing`` workload calls on ``device``."""
    return TimingContext(torch, device)


__all__ = [
    "timing",
    "timing_context",
    "TimingContext",
    "TimingRecord",
    "AcceleratorTimer",
]

"""Time accelerator workloads with stream-value-gated CUDA events.

This is the ``cuda_event_wait_value32_ffi`` method: a stream-value gate (see
:mod:`probing.timing.gates`) holds the start event, workload, and end event in
the queue until the host releases a device-side flag, so the measured window
excludes host enqueue latency. It requires a CUDA device that supports driver
stream memory operations; there is no fallback timer.
"""

from __future__ import annotations

from dataclasses import dataclass

from probing.timing.backend import event_pair, record_event
from probing.timing.gates import acquire_gate

METHOD = "cuda_event_wait_value32_ffi"


@dataclass(frozen=True)
class TimingRecord:
    """One measured workload call: elapsed time, method used, and return value."""

    elapsed_ms: float
    method: str
    value: object


class AcceleratorTimer:
    """Time workloads on a CUDA device using a stream-value gate."""

    def __init__(self, torch, device: int):
        self._torch = torch
        self._device = device
        self._gate = acquire_gate(torch, device)

    @property
    def method(self) -> str:
        return METHOD

    def run(self, fn, *args, **kwargs) -> TimingRecord:
        """Run one workload and return its :class:`TimingRecord`."""
        start, end, stream = event_pair(self._torch, self._device)
        self._gate.reset(stream)
        self._gate.block(stream)

        record_event(start, stream)
        try:
            value = fn(*args, **kwargs)
        except Exception:
            self._gate.release()
            raise
        record_event(end, stream)
        self._gate.release()
        end.synchronize()
        elapsed_ms = float(start.elapsed_time(end))
        return TimingRecord(elapsed_ms, METHOD, value)

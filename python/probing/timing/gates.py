"""CUDA stream-value gate that removes host enqueue latency from timing.

The gate stalls the compute stream behind a device-side flag so that the start
event, the workload, and the end event are all enqueued before any of them run.
The host then releases the flag, so the start->end window measures GPU execution
time without host enqueue latency. The wait lives on a private stream that the
compute stream waits on via an event, so the workload's own kernel launches
never block on the host.

The gate drives ``cuStreamWaitValue32`` / ``cuStreamWriteValue32`` through the
CUDA driver via ctypes (:mod:`probing.timing._cuda_runtime_ffi`). It owns
dedicated CUDA streams and a flag tensor, so it is built at most once per device
and reused across measured iterations.
"""

from __future__ import annotations

import threading

from probing.timing.backend import backend, dev_name


def _stream_handle(stream) -> int:
    return int(stream.cuda_stream)


class StreamValueGate:
    """Stream-value gate backed by libcuda via ctypes FFI."""

    def __init__(self, torch, device: int):
        if backend(torch) != "cuda":
            raise RuntimeError("StreamValueGate requires the CUDA backend")
        self._torch = torch
        self._device = device
        self._flag = torch.empty((), device=dev_name(device), dtype=torch.int32)
        self._wait_stream = torch.cuda.Stream(device=device)
        self._release_stream = torch.cuda.Stream(device=device)
        self._block_event = None

        from probing.timing import _cuda_runtime_ffi

        self._runtime = _cuda_runtime_ffi.load()
        probe_stream = torch.cuda.Stream(device=device)
        if not self._runtime.can_use_stream_mem_ops(
            device, _stream_handle(probe_stream), int(self._flag.data_ptr())
        ):
            try:
                probe_stream.synchronize()
            except Exception:
                pass
            raise RuntimeError(
                f"cuda:{device} does not support CUDA driver stream memory operations"
            )
        probe_stream.synchronize()

    def reset(self, stream) -> None:
        with self._torch.cuda.device(self._device), self._torch.cuda.stream(stream):
            self._flag.zero_()
        # Keep the host-side release write ordered after the reset memset.
        stream.synchronize()

    def block(self, stream, value: int = 1) -> None:
        self._runtime.wait_value32(
            _stream_handle(self._wait_stream), int(self._flag.data_ptr()), int(value)
        )
        self._block_event = self._torch.cuda.Event()
        self._block_event.record(self._wait_stream)
        stream.wait_event(self._block_event)

    def release(self, value: int = 1) -> None:
        self._runtime.write_value32(
            _stream_handle(self._release_stream),
            int(self._flag.data_ptr()),
            int(value),
        )


_GATE_CACHE: dict = {}
_GATE_CACHE_LOCK = threading.Lock()


def acquire_gate(torch, device: int) -> StreamValueGate:
    """Return a cached stream-value gate, building it at most once per device."""
    key = int(device)
    with _GATE_CACHE_LOCK:
        gate = _GATE_CACHE.get(key)
        if gate is None:
            gate = StreamValueGate(torch, device)
            _GATE_CACHE[key] = gate
        return gate

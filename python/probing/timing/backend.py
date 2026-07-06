"""Accelerator backend helpers used by the timing module.

Only the primitives the timer needs live here: backend detection, device
naming, and timing-event creation.
"""
from __future__ import annotations


def _torch():
    """Import ``torch`` and register the NPU backend if it is installed."""
    import torch

    try:
        import torch_npu  # noqa: F401  (registers the npu backend if present)
    except Exception:
        pass
    return torch


def backend(torch=None) -> str:
    """Return the active accelerator backend: ``'npu'``, ``'cuda'``, or ``'cpu'``."""
    torch = torch or _torch()
    if hasattr(torch, "npu") and torch.npu.is_available():
        return "npu"
    if torch.cuda.is_available():
        return "cuda"
    return "cpu"


def dev_name(device: int) -> str:
    """Return the ``backend:index`` string for ``device`` (``'cpu'`` on host)."""
    bk = backend()
    return f"{bk}:{device}" if bk != "cpu" else "cpu"


def _accelerator_module(torch):
    """Return the ``torch.cuda``/``torch.npu`` module, or ``None`` on CPU."""
    bk = backend(torch)
    if bk == "npu":
        return torch.npu
    if bk == "cuda":
        return torch.cuda
    return None


def event_pair(torch, device):
    """Create a ``(start, end, stream)`` timing triple on ``device``.

    Returns ``None`` when the backend has no timing events (e.g. CPU), which
    signals callers to fall back to a host-side timer.
    """
    accel = _accelerator_module(torch)
    if accel is None:
        return None

    event_cls = getattr(accel, "Event", None)
    current_stream = getattr(accel, "current_stream", None)
    if event_cls is None or current_stream is None:
        return None

    try:
        stream = current_stream(device)
    except TypeError:
        stream = current_stream()
    return event_cls(enable_timing=True), event_cls(enable_timing=True), stream


def record_event(event, stream) -> None:
    """Record ``event`` on ``stream``, tolerating stream-less event APIs."""
    try:
        event.record(stream)
    except TypeError:
        event.record()

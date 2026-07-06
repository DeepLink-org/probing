"""Unit tests for :mod:`probing.timing.backend`.

These exercise the accelerator backend helpers with lightweight fakes so they
run without a real CUDA/NPU device.
"""

from __future__ import annotations

from probing.timing import backend as backend_mod


class _FakeAccel:
    """Stand-in for ``torch.cuda`` / ``torch.npu``."""

    def __init__(self, available=True, event_cls=None, current_stream=None):
        self._available = available
        if event_cls is not None:
            self.Event = event_cls
        if current_stream is not None:
            self.current_stream = current_stream

    def is_available(self):
        return self._available


class _FakeTorch:
    """Minimal ``torch`` stand-in exposing only ``cuda`` / ``npu``."""

    def __init__(self, cuda=None, npu=None):
        if cuda is not None:
            self.cuda = cuda
        if npu is not None:
            self.npu = npu


class _FakeEvent:
    def __init__(self, enable_timing=False):
        self.enable_timing = enable_timing


def test_backend_prefers_npu_when_available():
    torch = _FakeTorch(cuda=_FakeAccel(available=True), npu=_FakeAccel(available=True))
    assert backend_mod.backend(torch) == "npu"


def test_backend_reports_cuda_when_no_npu():
    torch = _FakeTorch(cuda=_FakeAccel(available=True))
    assert backend_mod.backend(torch) == "cuda"


def test_backend_ignores_npu_that_is_unavailable():
    torch = _FakeTorch(cuda=_FakeAccel(available=True), npu=_FakeAccel(available=False))
    assert backend_mod.backend(torch) == "cuda"


def test_backend_falls_back_to_cpu():
    torch = _FakeTorch(cuda=_FakeAccel(available=False))
    assert backend_mod.backend(torch) == "cpu"


def test_dev_name_uses_backend_and_index(monkeypatch):
    monkeypatch.setattr(backend_mod, "backend", lambda torch=None: "cuda")
    assert backend_mod.dev_name(3) == "cuda:3"


def test_dev_name_is_plain_cpu_on_host(monkeypatch):
    monkeypatch.setattr(backend_mod, "backend", lambda torch=None: "cpu")
    assert backend_mod.dev_name(0) == "cpu"


def test_event_pair_returns_none_on_cpu():
    torch = _FakeTorch(cuda=_FakeAccel(available=False))
    assert backend_mod.event_pair(torch, 0) is None


def test_event_pair_returns_timing_triple_on_cuda():
    seen = {}

    def current_stream(device):
        seen["device"] = device
        return "STREAM"

    accel = _FakeAccel(available=True, event_cls=_FakeEvent, current_stream=current_stream)
    torch = _FakeTorch(cuda=accel)

    start, end, stream = backend_mod.event_pair(torch, 5)

    assert seen["device"] == 5
    assert stream == "STREAM"
    assert isinstance(start, _FakeEvent) and isinstance(end, _FakeEvent)
    assert start is not end
    assert start.enable_timing is True and end.enable_timing is True


def test_event_pair_tolerates_stream_without_device_arg():
    def current_stream(device=None):
        if device is not None:
            raise TypeError("no device arg")
        return "DEFAULT_STREAM"

    accel = _FakeAccel(available=True, event_cls=_FakeEvent, current_stream=current_stream)
    torch = _FakeTorch(cuda=accel)

    _start, _end, stream = backend_mod.event_pair(torch, 0)
    assert stream == "DEFAULT_STREAM"


def test_event_pair_returns_none_when_events_missing():
    accel = _FakeAccel(available=True)  # no Event / current_stream attributes
    torch = _FakeTorch(cuda=accel)
    assert backend_mod.event_pair(torch, 0) is None


def test_record_event_passes_stream():
    calls = []

    class _Event:
        def record(self, stream):
            calls.append(stream)

    backend_mod.record_event(_Event(), "STREAM")
    assert calls == ["STREAM"]


def test_record_event_falls_back_without_stream_arg():
    calls = []

    class _Event:
        def record(self, stream=None):
            if stream is not None:
                raise TypeError("stream not accepted")
            calls.append("no-stream")

    backend_mod.record_event(_Event(), "STREAM")
    assert calls == ["no-stream"]

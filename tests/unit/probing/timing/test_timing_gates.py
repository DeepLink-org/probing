"""Unit tests for :mod:`probing.timing.gates`.

The stream-value gate needs real CUDA driver stream memory ops to run, so these
tests cover the parts that can be exercised without a device: the CUDA-backend
guard, the stream-handle helper, and the per-device gate cache.
"""

from __future__ import annotations

import pytest

from probing.timing import gates


class _FakeAccel:
    def __init__(self, available):
        self._available = available

    def is_available(self):
        return self._available


class _FakeTorch:
    def __init__(self, cuda_available):
        self.cuda = _FakeAccel(cuda_available)


class _FakeStream:
    def __init__(self, handle):
        self.cuda_stream = handle


def test_stream_handle_reads_cuda_stream():
    assert gates._stream_handle(_FakeStream(1234)) == 1234


def test_stream_value_gate_requires_cuda_backend():
    torch = _FakeTorch(cuda_available=False)
    with pytest.raises(RuntimeError, match="requires the CUDA backend"):
        gates.StreamValueGate(torch, 0)


@pytest.fixture
def _clear_gate_cache():
    gates._GATE_CACHE.clear()
    yield
    gates._GATE_CACHE.clear()


def test_acquire_gate_builds_once_per_device(monkeypatch, _clear_gate_cache):
    built = []

    class _FakeGate:
        def __init__(self, torch, device):
            built.append(device)
            self.device = device

    monkeypatch.setattr(gates, "StreamValueGate", _FakeGate)

    torch = object()
    first = gates.acquire_gate(torch, 0)
    second = gates.acquire_gate(torch, 0)

    assert first is second
    assert built == [0]


def test_acquire_gate_is_per_device(monkeypatch, _clear_gate_cache):
    built = []

    class _FakeGate:
        def __init__(self, torch, device):
            built.append(device)
            self.device = device

    monkeypatch.setattr(gates, "StreamValueGate", _FakeGate)

    torch = object()
    gate0 = gates.acquire_gate(torch, 0)
    gate1 = gates.acquire_gate(torch, 1)

    assert gate0 is not gate1
    assert built == [0, 1]


def test_acquire_gate_coerces_device_key(monkeypatch, _clear_gate_cache):
    """``acquire_gate`` normalizes the device to ``int`` for its cache key."""
    built = []

    class _FakeGate:
        def __init__(self, torch, device):
            built.append(device)

    monkeypatch.setattr(gates, "StreamValueGate", _FakeGate)

    torch = object()
    first = gates.acquire_gate(torch, 0)
    second = gates.acquire_gate(torch, 0.0)  # same key after int() coercion

    assert first is second
    assert built == [0]

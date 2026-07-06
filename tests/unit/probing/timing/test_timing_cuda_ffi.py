"""Unit tests for :mod:`probing.timing._cuda_runtime_ffi`.

Loading ``libcuda`` requires a driver, so these tests cover the pure-Python
pieces: the symbol resolver (with its ``_v2`` fallback), the driver constants,
and the process-wide ``load()`` singleton.
"""

from __future__ import annotations

import pytest

from probing.timing import _cuda_runtime_ffi as ffi


class _FakeHandle:
    pass


def test_symbol_returns_named_attribute():
    handle = _FakeHandle()
    handle.cuStreamWaitValue32_v2 = "primary"
    assert ffi._symbol(handle, "cuStreamWaitValue32_v2", "cuStreamWaitValue32") == "primary"


def test_symbol_uses_fallback_when_primary_missing():
    handle = _FakeHandle()
    handle.cuStreamWaitValue32 = "fallback"
    resolved = ffi._symbol(handle, "cuStreamWaitValue32_v2", "cuStreamWaitValue32")
    assert resolved == "fallback"


def test_symbol_raises_when_symbol_absent():
    handle = _FakeHandle()
    with pytest.raises(RuntimeError, match="not available in the CUDA driver library"):
        ffi._symbol(handle, "cuMissing")


def test_driver_constants():
    assert ffi.CUDA_SUCCESS == 0
    assert ffi.CU_STREAM_WAIT_VALUE_GEQ == 0
    assert ffi.CU_STREAM_WRITE_VALUE_DEFAULT == 0
    assert ffi.CU_DEVICE_ATTRIBUTE_CAN_USE_STREAM_MEM_OPS_V1 == 92


def test_load_is_process_wide_singleton(monkeypatch):
    built = []

    class _FakeApi:
        def __init__(self):
            built.append(object())

    monkeypatch.setattr(ffi, "CudaDriverApi", _FakeApi)
    monkeypatch.setattr(ffi, "_RUNTIME", None)

    first = ffi.load()
    second = ffi.load()

    assert first is second
    assert len(built) == 1

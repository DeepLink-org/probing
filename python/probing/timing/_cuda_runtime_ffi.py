"""ctypes bridge for CUDA Driver stream memory operations.

Used by the FFI stream-value gate. Loads ``libcuda`` directly and exposes the
``cuStreamWaitValue32`` / ``cuStreamWriteValue32`` driver entry points so the
host can stall and release a device-side flag without compiling anything.
"""
from __future__ import annotations

import ctypes
import threading

_LOCK = threading.Lock()
_RUNTIME = None

CUDA_SUCCESS = 0
CU_STREAM_WAIT_VALUE_GEQ = 0
CU_STREAM_WRITE_VALUE_DEFAULT = 0
CU_DEVICE_ATTRIBUTE_CAN_USE_STREAM_MEM_OPS_V1 = 92


def _driver_library():
    """Load the CUDA Driver API library."""
    for name in ("libcuda.so.1", "libcuda.so"):
        try:
            return ctypes.CDLL(name)
        except OSError:
            continue
    raise RuntimeError("failed to load libcuda.so.1")


def _symbol(handle, name: str, fallback: str | None = None):
    sym = getattr(handle, name, None)
    if sym is None and fallback is not None:
        sym = getattr(handle, fallback, None)
    if sym is None:
        raise RuntimeError(f"{name} is not available in the CUDA driver library")
    return sym


class CudaDriverApi:
    """Small typed wrapper around the CUDA Driver C ABI."""

    def __init__(self):
        self._lib = _driver_library()
        self._bind()
        self._check(self._cu_init(0), "cuInit")

    def _bind(self) -> None:
        self._cu_init = _symbol(self._lib, "cuInit")
        self._cu_init.argtypes = [ctypes.c_uint]
        self._cu_init.restype = ctypes.c_int

        self._cu_get_error_name = _symbol(self._lib, "cuGetErrorName")
        self._cu_get_error_name.argtypes = [
            ctypes.c_int,
            ctypes.POINTER(ctypes.c_char_p),
        ]
        self._cu_get_error_name.restype = ctypes.c_int

        self._cu_get_error_string = _symbol(self._lib, "cuGetErrorString")
        self._cu_get_error_string.argtypes = [
            ctypes.c_int,
            ctypes.POINTER(ctypes.c_char_p),
        ]
        self._cu_get_error_string.restype = ctypes.c_int

        self._cu_device_get = _symbol(self._lib, "cuDeviceGet")
        self._cu_device_get.argtypes = [ctypes.POINTER(ctypes.c_int), ctypes.c_int]
        self._cu_device_get.restype = ctypes.c_int

        self._cu_device_get_attribute = _symbol(self._lib, "cuDeviceGetAttribute")
        self._cu_device_get_attribute.argtypes = [
            ctypes.POINTER(ctypes.c_int),
            ctypes.c_int,
            ctypes.c_int,
        ]
        self._cu_device_get_attribute.restype = ctypes.c_int

        self._cu_ctx_get_current = _symbol(self._lib, "cuCtxGetCurrent")
        self._cu_ctx_get_current.argtypes = [ctypes.POINTER(ctypes.c_void_p)]
        self._cu_ctx_get_current.restype = ctypes.c_int

        self._cu_ctx_get_device = _symbol(self._lib, "cuCtxGetDevice")
        self._cu_ctx_get_device.argtypes = [ctypes.POINTER(ctypes.c_int)]
        self._cu_ctx_get_device.restype = ctypes.c_int

        self._cu_device_primary_ctx_retain = _symbol(
            self._lib, "cuDevicePrimaryCtxRetain"
        )
        self._cu_device_primary_ctx_retain.argtypes = [
            ctypes.POINTER(ctypes.c_void_p),
            ctypes.c_int,
        ]
        self._cu_device_primary_ctx_retain.restype = ctypes.c_int

        self._cu_ctx_set_current = _symbol(self._lib, "cuCtxSetCurrent")
        self._cu_ctx_set_current.argtypes = [ctypes.c_void_p]
        self._cu_ctx_set_current.restype = ctypes.c_int

        self._cu_stream_wait_value32 = _symbol(
            self._lib, "cuStreamWaitValue32_v2", "cuStreamWaitValue32"
        )
        self._cu_stream_wait_value32.argtypes = [
            ctypes.c_void_p,
            ctypes.c_uint64,
            ctypes.c_uint32,
            ctypes.c_uint,
        ]
        self._cu_stream_wait_value32.restype = ctypes.c_int

        self._cu_stream_write_value32 = _symbol(
            self._lib, "cuStreamWriteValue32_v2", "cuStreamWriteValue32"
        )
        self._cu_stream_write_value32.argtypes = [
            ctypes.c_void_p,
            ctypes.c_uint64,
            ctypes.c_uint32,
            ctypes.c_uint,
        ]
        self._cu_stream_write_value32.restype = ctypes.c_int

    def _check(self, result: int, op: str) -> None:
        if result == CUDA_SUCCESS:
            return

        name = ctypes.c_char_p()
        message = ctypes.c_char_p()
        self._cu_get_error_name(result, ctypes.byref(name))
        self._cu_get_error_string(result, ctypes.byref(message))

        parts = [f"{op} failed"]
        if name.value:
            parts.append(name.value.decode("utf-8", "replace"))
        parts.append(f"({result})")
        if message.value:
            parts.append(message.value.decode("utf-8", "replace"))
        raise RuntimeError(": ".join(parts))

    def clear_error(self) -> None:
        """CUDA Driver calls return errors directly, so there is no sticky error."""

    def set_device(self, device: int) -> None:
        device = int(device)

        current = ctypes.c_void_p()
        self._check(
            self._cu_ctx_get_current(ctypes.byref(current)), "cuCtxGetCurrent"
        )
        if current.value is not None:
            current_device = ctypes.c_int()
            result = self._cu_ctx_get_device(ctypes.byref(current_device))
            if result == CUDA_SUCCESS and current_device.value == device:
                return

        cu_device = ctypes.c_int()
        self._check(self._cu_device_get(ctypes.byref(cu_device), device), "cuDeviceGet")

        primary = ctypes.c_void_p()
        self._check(
            self._cu_device_primary_ctx_retain(ctypes.byref(primary), cu_device.value),
            "cuDevicePrimaryCtxRetain",
        )
        self._check(self._cu_ctx_set_current(primary), "cuCtxSetCurrent")

    def _device_supports_stream_mem_ops(self, device: int) -> bool:
        cu_device = ctypes.c_int()
        self._check(self._cu_device_get(ctypes.byref(cu_device), device), "cuDeviceGet")

        supported = ctypes.c_int()
        self._check(
            self._cu_device_get_attribute(
                ctypes.byref(supported),
                CU_DEVICE_ATTRIBUTE_CAN_USE_STREAM_MEM_OPS_V1,
                cu_device.value,
            ),
            "cuDeviceGetAttribute(CAN_USE_STREAM_MEM_OPS)",
        )
        return supported.value != 0

    def wait_value32(self, stream_handle: int, ptr: int, value: int) -> None:
        self._check(
            self._cu_stream_wait_value32(
                ctypes.c_void_p(int(stream_handle)),
                ctypes.c_uint64(int(ptr)),
                ctypes.c_uint32(int(value)),
                ctypes.c_uint(CU_STREAM_WAIT_VALUE_GEQ),
            ),
            "cuStreamWaitValue32",
        )

    def write_value32(self, stream_handle: int, ptr: int, value: int) -> None:
        self._check(
            self._cu_stream_write_value32(
                ctypes.c_void_p(int(stream_handle)),
                ctypes.c_uint64(int(ptr)),
                ctypes.c_uint32(int(value)),
                ctypes.c_uint(CU_STREAM_WRITE_VALUE_DEFAULT),
            ),
            "cuStreamWriteValue32",
        )

    def can_use_stream_mem_ops(
        self,
        device: int,
        stream_handle: int,
        ptr: int,
    ) -> bool:
        """Probe stream memory ops on a private stream.

        The write is queued before the wait, so an unsupported wait cannot leave
        the stream blocked forever.
        """
        try:
            self.set_device(device)
            if not self._device_supports_stream_mem_ops(device):
                return False
            self.write_value32(stream_handle, ptr, 1)
            self.wait_value32(stream_handle, ptr, 1)
        except RuntimeError:
            self.clear_error()
            return False
        return True


def load() -> CudaDriverApi:
    """Load CUDA Driver symbols once per process."""
    global _RUNTIME
    with _LOCK:
        if _RUNTIME is None:
            _RUNTIME = CudaDriverApi()
        return _RUNTIME

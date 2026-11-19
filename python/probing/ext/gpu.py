"""Per-process GPU memory sampling via NVML.

The device-level ``gpu.utilization`` table cannot attribute memory to training
vs rollout processes when they share a GPU. This sampler resolves the current
process (and its children) to NVML PIDs and writes ``gpu.process_memory``.

Disabled by default: enable with ``PROBING_GPU_PROCESS_MEMORY=1``. Sampling
uses the same NVML calls as ``nvidia-smi``; keep the interval >= 5s on busy
training nodes.
"""

from __future__ import annotations

import os
import sys
import threading
import time
from typing import Any

FIELDS = ["ts", "pid", "device_id", "used_bytes", "name"]
_TABLE_NAME = "gpu.process_memory"

CUDA_FIELDS = [
    "ts",
    "pid",
    "device_id",
    "allocated_bytes",
    "reserved_bytes",
    "max_allocated_bytes",
    "max_reserved_bytes",
    "free_bytes",
    "total_bytes",
]
_CUDA_TABLE_NAME = "python.cuda_memory"

_started = False
_start_lock = threading.Lock()
_table: Any = None

_cuda_started = False
_cuda_start_lock = threading.Lock()
_cuda_table: Any = None


def _enabled() -> bool:
    value = os.environ.get("PROBING_GPU_PROCESS_MEMORY", "0").strip().lower()
    return value in {"1", "true", "yes", "on"}


def _interval_s() -> float:
    try:
        return max(
            float(os.environ.get("PROBING_GPU_PROCESS_MEMORY_INTERVAL", "5")), 1.0
        )
    except ValueError:
        return 5.0


def _get_table():
    global _table
    if _table is not None:
        return _table
    import probing

    try:
        _table = probing.ExternalTable.get_or_create(_TABLE_NAME, FIELDS)
    except Exception:
        try:
            _table = probing.ExternalTable.get(_TABLE_NAME)
        except Exception:
            _table = probing.ExternalTable(_TABLE_NAME, FIELDS)
    return _table


def _proc_ngid(pid: int) -> int | None:
    """Return the host-namespace PID for a container-local PID.

    NVML reports host PIDs in containers, while ``psutil`` and
    ``/proc/self`` use the container PID namespace. ``Ngid`` is the bridge.
    """

    try:
        with open(f"/proc/{pid}/status", encoding="utf-8") as handle:
            for line in handle:
                if not line.startswith("Ngid:"):
                    continue
                value = line.split(":", 1)[1].strip()
                ngid = int(value)
                return ngid if ngid > 0 else None
    except (FileNotFoundError, PermissionError, ProcessLookupError, ValueError):
        pass
    return None


def _local_pids() -> set[int]:
    pids = {os.getpid()}
    try:
        import psutil

        process = psutil.Process()
        pids.update(child.pid for child in process.children(recursive=True))
    except Exception:
        pass
    expanded = set(pids)
    for pid in pids:
        ngid = _proc_ngid(pid)
        if ngid is not None:
            expanded.add(ngid)
    return expanded


def _nvml_process_getter(pynvml):
    for name in (
        "nvmlDeviceGetComputeRunningProcesses_v3",
        "nvmlDeviceGetComputeRunningProcesses_v2",
        "nvmlDeviceGetComputeRunningProcesses",
    ):
        getter = getattr(pynvml, name, None)
        if getter is not None:
            return getter
    return None


def _loop() -> None:
    try:
        import pynvml
    except Exception:
        return

    try:
        pynvml.nvmlInit()
    except Exception:
        return

    try:
        getter = _nvml_process_getter(pynvml)
        if getter is None:
            return
        device_count = pynvml.nvmlDeviceGetCount()
        interval = _interval_s()
        while True:
            pids = _local_pids()
            timestamp = time.time_ns() // 1000
            for device_id in range(device_count):
                try:
                    handle = pynvml.nvmlDeviceGetHandleByIndex(device_id)
                    name = pynvml.nvmlDeviceGetName(handle)
                    if isinstance(name, bytes):
                        name = name.decode("utf-8", "replace")
                    for proc in getter(handle):
                        pid = int(getattr(proc, "pid", 0) or 0)
                        if pid not in pids:
                            continue
                        used = getattr(proc, "usedGpuMemory", None)
                        if used is None:
                            continue
                        used = int(used)
                        if used <= 0:
                            continue
                        _get_table().append(
                            [timestamp, pid, device_id, used, str(name)]
                        )
                except Exception:
                    continue
            time.sleep(interval)
    finally:
        try:
            pynvml.nvmlShutdown()
        except Exception:
            pass


def start() -> bool:
    """Start the per-process GPU memory sampler when explicitly enabled."""

    global _started
    if not _enabled():
        return False
    with _start_lock:
        if _started:
            return True
        _get_table()
        thread = threading.Thread(
            target=_loop, name="probing-gpu-process-memory", daemon=True
        )
        thread.start()
        _started = True
        return True


def _cuda_enabled() -> bool:
    value = os.environ.get("PROBING_CUDA_MEMORY", "1").strip().lower()
    return value not in {"0", "false", "no", "off"}


def _cuda_interval_s() -> float:
    try:
        return max(float(os.environ.get("PROBING_CUDA_MEMORY_INTERVAL", "2")), 1.0)
    except ValueError:
        return 2.0


def _get_cuda_table():
    global _cuda_table
    if _cuda_table is not None:
        return _cuda_table
    import probing

    try:
        _cuda_table = probing.ExternalTable.get_or_create(_CUDA_TABLE_NAME, CUDA_FIELDS)
    except Exception:
        try:
            _cuda_table = probing.ExternalTable.get(_CUDA_TABLE_NAME)
        except Exception:
            _cuda_table = probing.ExternalTable(_CUDA_TABLE_NAME, CUDA_FIELDS)
    return _cuda_table


def _cuda_loop() -> None:
    """Sample the torch allocator without importing torch ourselves.

    The site hook runs before the training script imports torch, so wait until
    it appears in ``sys.modules`` (or give up after a few minutes).
    """

    deadline = time.time() + 300
    torch = None
    while time.time() < deadline:
        torch = sys.modules.get("torch")
        if torch is not None:
            break
        time.sleep(1)
    if torch is None:
        return
    try:
        if not torch.cuda.is_available():
            return
    except Exception:
        return

    interval = _cuda_interval_s()
    while True:
        timestamp = time.time_ns() // 1000
        try:
            device_count = torch.cuda.device_count()
        except Exception:
            return
        for device_id in range(device_count):
            try:
                allocated = int(torch.cuda.memory_allocated(device_id))
                reserved = int(torch.cuda.memory_reserved(device_id))
                max_allocated = int(torch.cuda.max_memory_allocated(device_id))
                max_reserved = int(torch.cuda.max_memory_reserved(device_id))
                free, total = torch.cuda.mem_get_info(device_id)
                _get_cuda_table().append(
                    [
                        timestamp,
                        os.getpid(),
                        device_id,
                        allocated,
                        reserved,
                        max_allocated,
                        max_reserved,
                        int(free),
                        int(total),
                    ]
                )
            except Exception:
                continue
        time.sleep(interval)


def start_cuda_memory() -> bool:
    """Start the torch allocator sampler (default on; disable with PROBING_CUDA_MEMORY=0)."""

    global _cuda_started
    if not _cuda_enabled():
        return False
    with _cuda_start_lock:
        if _cuda_started:
            return True
        _get_cuda_table()
        thread = threading.Thread(
            target=_cuda_loop, name="probing-cuda-memory", daemon=True
        )
        thread.start()
        _cuda_started = True
        return True


__all__ = ["CUDA_FIELDS", "FIELDS", "start", "start_cuda_memory"]

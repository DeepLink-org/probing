"""Low-overhead Python GC telemetry.

The sampler records generation counts plus per-collection pause durations
through ``gc.callbacks``. It is started by the site hook when probing is
enabled, and writes one row per second to the ``python.gc`` memtable.

The callback itself only appends to a bounded deque; all formatting and table
I/O happens on the sampler thread.
"""

from __future__ import annotations

import gc
import json
import os
import threading
import time
from collections import deque
from typing import Any

FIELDS = [
    "ts",
    "pid",
    "gen0_count",
    "gen1_count",
    "gen2_count",
    "gen0_collections",
    "gen1_collections",
    "gen2_collections",
    "gen0_collected",
    "gen1_collected",
    "gen2_collected",
    "gen2_uncollectable",
    "max_pause_ms",
    "last_pause_ms",
    "last_pause_gen",
    "pause_count",
    "pause_histogram_json",
]

_TABLE_NAME = "python.gc"
_MAX_PAUSES = 4096

_started = False
_start_lock = threading.Lock()
_pause_lock = threading.Lock()
_pauses: deque[tuple[int, float]] = deque(maxlen=_MAX_PAUSES)
_tls = threading.local()
_table: Any = None


def _enabled() -> bool:
    value = os.environ.get("PROBING_GC", "1").strip().lower()
    return value not in {"0", "false", "no", "off"}


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


def _callback(phase: str, info: dict[str, Any]) -> None:
    if phase == "start":
        _tls.started_at = time.perf_counter()
        return
    started_at = getattr(_tls, "started_at", None)
    if started_at is None:
        return
    elapsed_ms = (time.perf_counter() - started_at) * 1000.0
    generation = int(info.get("generation", -1))
    with _pause_lock:
        _pauses.append((generation, elapsed_ms))


def _snapshot() -> tuple[tuple[int, int, int], list[dict[str, Any]]]:
    return gc.get_count(), gc.get_stats()


def _loop() -> None:
    while True:
        time.sleep(1.0)
        try:
            with _pause_lock:
                pauses = list(_pauses)
                _pauses.clear()
            counts, stats = _snapshot()
            histogram: dict[str, int] = {}
            for generation, _ in pauses:
                key = str(generation)
                histogram[key] = histogram.get(key, 0) + 1
            last_gen, last_ms = pauses[-1] if pauses else (-1, 0.0)
            row = [
                time.time_ns() // 1000,
                os.getpid(),
                counts[0],
                counts[1],
                counts[2],
                stats[0].get("collections", 0),
                stats[1].get("collections", 0),
                stats[2].get("collections", 0),
                stats[0].get("collected", 0),
                stats[1].get("collected", 0),
                stats[2].get("collected", 0),
                stats[2].get("uncollectable", 0),
                max((ms for _, ms in pauses), default=0.0),
                last_ms,
                last_gen,
                len(pauses),
                json.dumps(histogram, separators=(",", ":")),
            ]
            _get_table().append(row)
        except Exception:
            # Telemetry must never break the training process.
            pass


def start() -> bool:
    """Start the GC sampler if enabled. Returns True when it is running."""

    global _started
    if not _enabled():
        return False
    with _start_lock:
        if _started:
            return True
        _get_table()
        gc.callbacks.append(_callback)
        thread = threading.Thread(target=_loop, name="probing-gc-sampler", daemon=True)
        thread.start()
        _started = True
        return True


def stop() -> None:
    """Remove the callback (the daemon thread exits with the process)."""

    global _started
    with _start_lock:
        if not _started:
            return
        try:
            gc.callbacks.remove(_callback)
        except ValueError:
            pass
        _started = False


__all__ = ["FIELDS", "start", "stop"]

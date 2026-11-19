"""Install an on-demand / GC-triggered host-memory snapshot hook.

Usage (in an already-running Python process that has probing attached):

    probing -t <pid> eval "exec(open('/path/to/gc_mem_hook.py').read())"

The hook is idempotent and re-entrancy safe. It writes one row to
``rl.mem_snapshot`` on install and after every explicit ``gc.collect()`` call.
"""

import gc
import os
import time

import psutil
import probing

FIELDS = [
    "ts",
    "pid",
    "reason",
    "rss_bytes",
    "uss_bytes",
    "pss_bytes",
    "anon_bytes",
    "threads",
    "gc_generation",
]

_TABLE = probing.ExternalTable.get_or_create("rl.mem_snapshot", FIELDS)
_IN_SNAPSHOT = False


def snapshot(reason="manual", generation=-1):
    """Append one current-process memory row (RSS/USS/PSS)."""

    global _IN_SNAPSHOT
    if _IN_SNAPSHOT:
        return
    _IN_SNAPSHOT = True
    try:
        proc = psutil.Process()
        info = proc.memory_full_info()
        _TABLE.append(
            [
                time.time_ns() // 1000,
                os.getpid(),
                reason,
                info.rss,
                info.uss,
                info.pss,
                getattr(info, "anon", 0),
                proc.num_threads(),
                generation,
            ]
        )
    finally:
        _IN_SNAPSHOT = False


if not getattr(gc, "_probing_mem_hook_installed", False):
    _ORIG_COLLECT = gc.collect

    def collect_and_snapshot(*args, **kwargs):
        generation = args[0] if args else -1
        result = _ORIG_COLLECT(*args, **kwargs)
        snapshot("gc.collect", generation)
        return result

    gc.collect = collect_and_snapshot
    gc._probing_mem_hook_installed = True

    # Optional: also sample every automatic GC (gen0/gen1/gen2).
    # Uncomment before installing the hook.
    #
    # def _gc_callback(phase, info):
    #     if phase == "stop":
    #         snapshot("gc.auto", info.get("generation", -1))
    #
    # gc.callbacks.append(_gc_callback)

snapshot("hook_installed")
print("gc memory hook installed", flush=True)

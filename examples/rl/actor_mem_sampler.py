"""Sample local Ray actors' RSS/USS/PSS into ``rl.actor_memory``.

Run this inside the **driver** process so the driver's Web UI can show all Ray
actors on the same node:

    probing -t <driver_pid> eval "exec(open('/path/to/actor_mem_sampler.py').read())"

For multi-node jobs, run the same snippet on each node (one driver/agent per
node) or federate the per-node probing endpoints.

The primary source is the Ray dashboard/state API
(``ray.util.state.list_actors``). If it returns no actors -- for example while
actors are still initializing, or when dashboard state is incomplete -- the
sampler falls back to scanning local ``ray::`` worker processes with psutil.
The RL launch script starts Ray with ``--include-dashboard=true``.
"""

import os
import threading
import time

import psutil
import probing

from probing.ext.ray import RAY_ACTOR_INFRA_CLASSES, actor_class_from_process_title

FIELDS = [
    "ts",
    "pid",
    "actor_id",
    "actor_class",
    "actor_name",
    "state",
    "rss_bytes",
    "uss_bytes",
    "pss_bytes",
    "threads",
]

_TABLE = probing.ExternalTable.get_or_create("rl.actor_memory", FIELDS)


def _get(obj, *names):
    for name in names:
        if isinstance(obj, dict) and name in obj:
            return obj[name]
        value = getattr(obj, name, None)
        if value is not None:
            return value
    return None


def _interval_s():
    try:
        return max(float(os.environ.get("PROBING_ACTOR_MEMORY_INTERVAL", "5")), 1.0)
    except ValueError:
        return 5.0


def _append_sample(
    proc,
    timestamp,
    *,
    actor_id="",
    actor_class="",
    actor_name="",
    state="ALIVE",
):
    try:
        info = proc.memory_full_info()
        threads = proc.num_threads()
    except (psutil.NoSuchProcess, psutil.AccessDenied):
        return False
    _TABLE.append(
        [
            timestamp,
            int(proc.pid),
            str(actor_id or ""),
            str(actor_class or ""),
            str(actor_name or ""),
            str(state or ""),
            info.rss,
            info.uss,
            info.pss,
            threads,
        ]
    )
    return True


def _scan_processes(timestamp):
    """Fallback: discover ``ray::`` actor processes when the state API is empty."""

    rows = 0
    for proc in psutil.process_iter(["pid", "cmdline"]):
        cmdline = proc.info.get("cmdline") or []
        title = cmdline[0] if cmdline else ""
        actor_class = actor_class_from_process_title(title)
        if not actor_class or actor_class in RAY_ACTOR_INFRA_CLASSES:
            continue
        if _append_sample(proc, timestamp, actor_class=actor_class):
            rows += 1
    return rows


def sample_once():
    from ray.util.state import list_actors

    timestamp = time.time_ns() // 1000
    try:
        actors = list_actors(
            detail=True,
            filters=[("state", "=", "ALIVE")],
            limit=10000,
        )
    except Exception:
        actors = []

    rows = 0
    for actor in actors:
        pid = _get(actor, "pid", "Pid")
        if not pid:
            continue
        try:
            proc = psutil.Process(int(pid))
        except (psutil.NoSuchProcess, psutil.AccessDenied, ValueError):
            # Remote-node actor or already exited.
            continue
        if _append_sample(
            proc,
            timestamp,
            actor_id=_get(actor, "actor_id", "ActorId"),
            actor_class=_get(actor, "class_name", "ActorClassName"),
            actor_name=_get(actor, "name", "Name"),
            state=_get(actor, "state", "State"),
        ):
            rows += 1

    if rows == 0:
        rows = _scan_processes(timestamp)
    return rows


def _loop():
    while True:
        try:
            sample_once()
        except Exception:
            # Telemetry must never break the driver.
            pass
        time.sleep(_interval_s())


if not getattr(probing, "_actor_mem_sampler_started", False):
    probing._actor_mem_sampler_started = True
    threading.Thread(target=_loop, name="probing-actor-memory", daemon=True).start()

print("actor memory sampler installed", flush=True)

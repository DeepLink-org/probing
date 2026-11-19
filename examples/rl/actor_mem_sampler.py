"""Sample local Ray actors' RSS/USS/PSS into ``rl.actor_memory``.

Run this inside the **driver** process so the driver's Web UI can show all Ray
actors on the same node:

    probing -t <driver_pid> eval "exec(open('/path/to/actor_mem_sampler.py').read())"

For multi-node jobs, run the same snippet on each node (one driver/agent per
node) or federate the per-node probing endpoints.

Requires the Ray dashboard/state API (``ray.util.state.list_actors``). The RL
launch script starts Ray with ``--include-dashboard=true``; if your cluster has
the dashboard disabled, enable it or use ``ray.nodes``/external psutil instead.
"""

import os
import threading
import time

import psutil
import probing

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


def sample_once():
    from ray.util.state import list_actors

    timestamp = time.time_ns() // 1000
    actors = list_actors(
        detail=True,
        filters=[("state", "=", "ALIVE")],
        limit=10000,
    )
    for actor in actors:
        pid = _get(actor, "pid", "Pid")
        if not pid:
            continue
        try:
            proc = psutil.Process(int(pid))
            info = proc.memory_full_info()
        except (psutil.NoSuchProcess, psutil.AccessDenied, ValueError):
            # Remote-node actor or already exited.
            continue
        _TABLE.append(
            [
                timestamp,
                int(pid),
                str(_get(actor, "actor_id", "ActorId") or ""),
                str(_get(actor, "class_name", "ActorClassName") or ""),
                str(_get(actor, "name", "Name") or ""),
                str(_get(actor, "state", "State") or ""),
                info.rss,
                info.uss,
                info.pss,
                proc.num_threads(),
            ]
        )


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

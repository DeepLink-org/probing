"""Structured snapshots of PyTorch wait counters and rendezvous state.

PyTorch exposes both capabilities through experimental/private APIs.  Keep all
version checks here so the HTTP and Web layers receive an explicit capability
state instead of treating unsupported runtimes as empty data.
"""

from __future__ import annotations

import json
import os
import threading
import urllib.request
from datetime import timedelta
from typing import Any, Callable

_worker_lock = threading.Lock()
_worker: Any = None
_wait_counter_provider_lock = threading.Lock()
_wait_counter_provider: tuple[str, Callable[[], dict[str, Any]]] | None = None


def register_wait_counter_provider(
    provider: Callable[[], dict[str, Any]], *, source: str = "custom"
) -> None:
    """Register a fallback provider for runtimes without PyTorch's debug handler."""
    if not callable(provider):
        raise TypeError("wait counter provider must be callable")
    normalized_source = source.strip() or "custom"
    global _wait_counter_provider
    with _wait_counter_provider_lock:
        _wait_counter_provider = (normalized_source, provider)


def unregister_wait_counter_provider(provider: Callable[[], dict[str, Any]]) -> None:
    global _wait_counter_provider
    with _wait_counter_provider_lock:
        if _wait_counter_provider is not None and _wait_counter_provider[1] is provider:
            _wait_counter_provider = None


def _rank() -> int:
    try:
        return int(os.environ.get("RANK", "0"))
    except ValueError:
        return 0


def _counter_category(name: str) -> str:
    lowered = name.lower()
    if "tcpstore" in lowered:
        return "tcpstore"
    if "processgroupnccl" in lowered or "nccl" in lowered:
        return "nccl"
    if "processgroup" in lowered:
        return "process_group"
    return "runtime"


def _normalize_wait_counters(
    payload: dict[str, Any], rank: int
) -> list[dict[str, Any]]:
    rows = []
    for name, raw in sorted(payload.items()):
        if not isinstance(raw, dict):
            continue
        calls = int(raw.get("total_calls") or 0)
        total = int(raw.get("total_time_us") or 0)
        rows.append(
            {
                "name": str(name),
                "category": _counter_category(str(name)),
                "rank": rank,
                "active_count": int(raw.get("active_count") or 0),
                "total_calls": calls,
                "total_time_us": total,
                "max_time_us": int(raw.get("max_time_us") or 0),
                "avg_time_us": total / calls if calls else 0.0,
            }
        )
    return rows


def _local_worker_url() -> str:
    global _worker
    with _worker_lock:
        if _worker is None:
            # Import registers C++/Python debug handlers as a side effect.  Bind
            # to loopback only; Probing never starts PyTorch's public frontend.
            import torch.distributed.debug._handlers  # noqa: F401
            from torch._C._distributed_c10d import _WorkerServer

            _worker = _WorkerServer("127.0.0.1", 0)
        return f"http://127.0.0.1:{int(_worker.port)}"


def _snapshot_pytorch_wait_counters() -> dict[str, Any]:
    request = urllib.request.Request(
        f"{_local_worker_url()}/handler/wait_counter_values",
        data=b"",
        method="POST",
    )
    with urllib.request.urlopen(request, timeout=2.0) as response:
        payload = json.loads(response.read().decode("utf-8"))
    if not isinstance(payload, dict):
        raise RuntimeError("wait counter handler returned a non-object payload")
    return payload


def _registered_wait_counter_provider() -> (
    tuple[str, Callable[[], dict[str, Any]]] | None
):
    with _wait_counter_provider_lock:
        return _wait_counter_provider


def snapshot_wait_counters() -> dict[str, Any]:
    try:
        payload = _snapshot_pytorch_wait_counters()
        return {
            "available": True,
            "error": None,
            "source": "pytorch",
            "rank": _rank(),
            "counters": _normalize_wait_counters(payload, _rank()),
        }
    except Exception as native_error:
        registered = _registered_wait_counter_provider()
        if registered is not None:
            source, provider = registered
            try:
                payload = provider()
                if not isinstance(payload, dict):
                    raise RuntimeError(
                        "registered provider returned a non-object payload"
                    )
                return {
                    "available": True,
                    "error": None,
                    "source": source,
                    "rank": _rank(),
                    "counters": _normalize_wait_counters(payload, _rank()),
                }
            except Exception as provider_error:
                native_error = RuntimeError(
                    f"native handler: {native_error}; {source}: {provider_error}"
                )
        return {
            "available": False,
            "error": f"PyTorch wait counters unavailable: {native_error}",
            "source": "unavailable",
            "rank": _rank(),
            "counters": [],
        }


def _store_category(key: str) -> str:
    lowered = key.lstrip("/").lower()
    if lowered.startswith("debug_server/rank"):
        return "debug_worker"
    if "torchelastic/assigned_ranks" in lowered:
        return "rank_assignment"
    if "torchelastic/role_info" in lowered:
        return "role"
    if "probing/torchrun/" in lowered:
        return "probing"
    if "default_pg" in lowered or "process_group" in lowered:
        return "process_group"
    if lowered.startswith("torch.rendezvous."):
        return "rendezvous"
    return "other"


def _value_preview(value: bytes, limit: int = 160) -> str:
    text = value.decode("utf-8", errors="backslashreplace")
    text = text.encode("unicode_escape").decode("ascii")
    return text if len(text) <= limit else f"{text[:limit]}…"


def _snapshot_tcpstore(store: Any, include_values: bool) -> list[dict[str, Any]]:
    keys = sorted(str(key) for key in store.list_keys())
    values = store.multi_get(keys)
    return _store_rows(keys, values, include_values)


def _store_rows(
    keys: list[str], values: list[Any], include_values: bool
) -> list[dict[str, Any]]:
    rows = []
    safe_categories = {"debug_worker", "rank_assignment", "role", "probing"}
    for key, value in zip(keys, values):
        raw = bytes(value)
        category = _store_category(key)
        visible = include_values or category in safe_categories
        rows.append(
            {
                "key": key,
                "category": category,
                "value_size": len(raw),
                "value_preview": _value_preview(raw) if visible else "",
                "redacted": not visible,
            }
        )
    return rows


def _env_int(name: str) -> int | None:
    try:
        value = os.environ.get(name)
        return int(value) if value not in (None, "") else None
    except ValueError:
        return None


def _tcpstore_facts(store: Any) -> list[dict[str, str]]:
    facts: list[dict[str, str]] = []
    host = str(getattr(store, "host", os.environ.get("MASTER_ADDR", ""))).strip()
    port = getattr(store, "port", os.environ.get("MASTER_PORT", ""))
    endpoint = f"{host}:{port}" if host and str(port).strip() else host
    backend = type(store).__name__
    facts.append(
        {
            "label": "Store",
            "value": f"{backend} · {endpoint}" if endpoint else backend,
        }
    )

    run_id = (
        os.environ.get("TORCHELASTIC_RUN_ID") or os.environ.get("RDZV_ID") or ""
    ).strip()
    if run_id:
        facts.append({"label": "Run", "value": run_id})

    rank = _env_int("RANK")
    world = _env_int("WORLD_SIZE")
    local_rank = _env_int("LOCAL_RANK")
    local_world = _env_int("LOCAL_WORLD_SIZE")
    if rank is not None or world is not None:
        value = f"{rank if rank is not None else '?'} / {world if world is not None else '?'}"
        if local_rank is not None or local_world is not None:
            value += f" · local {local_rank if local_rank is not None else '?'} / {local_world if local_world is not None else '?'}"
        facts.append({"label": "Rank", "value": value})

    node_rank = _env_int("GROUP_RANK")
    if node_rank is None:
        node_rank = _env_int("NODE_RANK")
    node_count = _env_int("GROUP_WORLD_SIZE")
    if node_count is None and world is not None and local_world:
        node_count = (world + local_world - 1) // local_world
    if node_rank is not None or node_count is not None:
        facts.append(
            {
                "label": "Node",
                "value": f"{node_rank if node_rank is not None else '?'} / {node_count if node_count is not None else '?'}",
            }
        )

    role = os.environ.get("ROLE_NAME", "").strip()
    role_rank = _env_int("ROLE_RANK")
    role_world = _env_int("ROLE_WORLD_SIZE")
    if role or role_rank is not None or role_world is not None:
        role_position = f"{role_rank if role_rank is not None else '?'} / {role_world if role_world is not None else '?'}"
        facts.append(
            {
                "label": "Role",
                "value": f"{role} · {role_position}" if role else role_position,
            }
        )

    restarts = _env_int("TORCHELASTIC_RESTART_COUNT")
    max_restarts = _env_int("TORCHELASTIC_MAX_RESTARTS")
    if restarts is not None or max_restarts is not None:
        facts.append(
            {
                "label": "Restart",
                "value": f"{restarts if restarts is not None else '?'} / {max_restarts if max_restarts is not None else '?'}",
            }
        )
    return facts


def _known_tcpstore_keys() -> list[str]:
    run_id = (
        os.environ.get("TORCHELASTIC_RUN_ID") or os.environ.get("RDZV_ID") or ""
    ).strip()
    group_world = _env_int("GROUP_WORLD_SIZE") or 0
    # Keep a malformed environment from turning a diagnostics request into a
    # large scan. Store.check() is non-blocking, but it still performs I/O.
    group_world = min(max(group_world, 0), 512)
    candidates: list[str] = []
    if run_id:
        candidates.extend(
            [
                f"torch.rendezvous.{run_id}",
                f"probing/torchrun/{run_id}/master",
            ]
        )
    rank = _env_int("RANK")
    if rank is not None:
        candidates.append(f"debug_server/rank{rank}")

    restart = _env_int("TORCHELASTIC_RESTART_COUNT") or 0
    dynamic_rounds = {0, restart, restart + 1}
    for index in range(group_world):
        elastic_keys = [
            f"torchelastic/role_info/{index}",
            f"torchelastic/assigned_ranks/{index}",
        ]
        candidates.extend(elastic_keys)
        if run_id:
            candidates.append(f"probing/torchrun/{run_id}/node/{index}/local0")
            # Static rendezvous uses PrefixStore(run_id); dynamic rendezvous
            # uses PrefixStore(torch.rendezvous.<run_id>.<round>).
            candidates.extend(f"{run_id}/{key}" for key in elastic_keys)
            for round_index in dynamic_rounds:
                candidates.extend(
                    f"torch.rendezvous.{run_id}.{round_index}/{key}"
                    for key in elastic_keys
                )
    return sorted(set(candidates))


def _snapshot_known_tcpstore(store: Any, include_values: bool) -> list[dict[str, Any]]:
    keys = []
    for key in _known_tcpstore_keys():
        try:
            if store.check([key]):
                keys.append(key)
        except (AttributeError, RuntimeError, TimeoutError, ValueError):
            continue
    if not keys:
        return []
    try:
        values = store.multi_get(keys)
    except (AttributeError, RuntimeError, TimeoutError, ValueError):
        return []
    return _store_rows(keys, values, include_values)


def _tcpstore_client() -> Any:
    try:
        from torch.distributed.debug._store import tcpstore_client

        return tcpstore_client(prefix="")
    except (ImportError, AttributeError):
        import torch.distributed as dist

        host = os.environ["MASTER_ADDR"]
        port = int(os.environ["MASTER_PORT"])
        return dist.TCPStore(
            host_name=host,
            port=port,
            is_master=False,
            timeout=timedelta(seconds=2),
            wait_for_workers=False,
        )


def snapshot_tcpstore(include_values: bool = False) -> dict[str, Any]:
    try:
        allow_values = include_values and os.environ.get(
            "PROBING_TCPSTORE_INSPECT", ""
        ).strip().lower() in {"1", "true", "yes", "on"}
        store = _tcpstore_client()
        catalog_available = hasattr(store, "list_keys")
        entries = (
            _snapshot_tcpstore(store, allow_values)
            if catalog_available
            else _snapshot_known_tcpstore(store, allow_values)
        )
        total_keys = len(entries) if catalog_available else int(store.num_keys())
        return {
            "available": True,
            "error": None,
            "values_enabled": allow_values,
            "catalog_available": catalog_available,
            "catalog_mode": "native" if catalog_available else "known_keys",
            "total_keys": total_keys,
            "identified_keys": len(entries),
            "facts": _tcpstore_facts(store),
            "entries": entries,
        }
    except Exception as exc:
        return {
            "available": False,
            "error": f"TCPStore inspection unavailable: {exc}",
            "values_enabled": False,
            "catalog_available": False,
            "catalog_mode": "unavailable",
            "total_keys": 0,
            "identified_keys": 0,
            "facts": [],
            "entries": [],
        }


def snapshot_runtime_debug(include_values: bool = False) -> dict[str, Any]:
    return {
        "wait_counters": snapshot_wait_counters(),
        "tcpstore": snapshot_tcpstore(include_values=include_values),
    }

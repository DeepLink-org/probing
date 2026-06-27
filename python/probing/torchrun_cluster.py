"""
Torchrun cluster registration for probing via the rendezvous TCPStore.

Rank 0 binds the local probing HTTP server, publishes the reachable master
address to the job store, and other ranks ``wait`` then ``PUT /apis/nodes``.

Requires ``torch.distributed.init_process_group()`` (e.g. ``torchrun``) before
``setup_torchrun_cluster()``.

Env:
- ``PROBING_TORCHRUN_CLUSTER``: set to ``0`` to disable (default: enable when WORLD_SIZE>1).
- ``PROBING_PORT``: optional listen port for rank 0; other ranks use ephemeral ports.
- ``PROBING_TORCHRUN_STORE_TIMEOUT``: seconds to wait for master key (default 120).
"""

from __future__ import annotations

import json
import logging
import os
import socket
import time
import urllib.error
import urllib.request
from datetime import timedelta
from typing import Any

logger = logging.getLogger(__name__)

_SETUP_DONE = False
_MASTER_KEY_SUFFIX = "master"
_MASTER_INFO: dict[str, str] | None = None


def _enabled() -> bool:
    if os.environ.get("PROBING_TORCHRUN_CLUSTER", "1").strip().lower() in (
        "0",
        "false",
        "no",
    ):
        return False
    return True


def _env_int(name: str, default: int = -1) -> int:
    raw = os.environ.get(name)
    if raw is None or raw == "":
        return default
    return int(raw)


def _run_prefix() -> str:
    run_id = (
        os.environ.get("TORCHELASTIC_RUN_ID")
        or os.environ.get("RDZV_ID")
        or os.environ.get("MASTER_PORT")
        or "run"
    )
    return f"probing/torchrun/{run_id}"


def _master_store_key() -> str:
    return f"{_run_prefix()}/{_MASTER_KEY_SUFFIX}"


def _rendezvous_store():
    import torch.distributed as dist
    import torch.distributed.distributed_c10d as c10d

    if not dist.is_initialized():
        raise RuntimeError(
            "torch.distributed must be initialized before probing torchrun cluster setup"
        )
    return c10d._get_default_store()


def _store_get_str(store, key: str) -> str:
    val = store.get(key)
    if isinstance(val, bytes):
        return val.decode()
    return str(val)


def _reachable_addr(bound: str) -> str:
    """Map a local bind address (e.g. 0.0.0.0:port) to an address peers can use."""
    if ":" not in bound:
        return bound
    host, port = bound.rsplit(":", 1)
    host = host.strip().strip("[]")
    if host in ("0.0.0.0", "::", "", "*"):
        master = os.environ.get("MASTER_ADDR", "").strip()
        if master in ("127.0.0.1", "localhost"):
            host = master
        else:
            host = master or socket.gethostname()
    return f"{host}:{port}"


def _bind_spec() -> str:
    rank = _env_int("RANK", 0)
    port = os.environ.get("PROBING_PORT", "").strip()
    if rank == 0 and port:
        return f"0.0.0.0:{port}"
    return "0.0.0.0:0"


def _ensure_http_server() -> str:
    """Start or reuse the probing HTTP server; return the bound ``host:port``."""
    from probing import config

    existing = config.get_str("server.address")
    if existing:
        return existing

    bind = _bind_spec()
    config.write("probing.server.address", bind)

    deadline = time.monotonic() + 10.0
    while time.monotonic() < deadline:
        addr = config.get_str("server.address")
        if addr and ":" in addr:
            return addr
        time.sleep(0.05)

    raise TimeoutError(f"probing HTTP server did not bind (requested {bind})")


def _optional_env_int(name: str) -> int | None:
    raw = os.environ.get(name)
    if raw is None or raw == "":
        return None
    return int(raw)


def _node_payload(local_addr: str) -> dict[str, Any]:
    from probing.parallel import current_role

    host = socket.gethostname()
    return {
        "host": host,
        "addr": local_addr,
        "local_rank": _optional_env_int("LOCAL_RANK"),
        "rank": _optional_env_int("RANK"),
        "world_size": _optional_env_int("WORLD_SIZE"),
        "group_rank": _optional_env_int("GROUP_RANK"),
        "group_world_size": _optional_env_int("GROUP_WORLD_SIZE"),
        "role_name": os.environ.get("ROLE_NAME"),
        "role_rank": _optional_env_int("ROLE_RANK"),
        "role_world_size": _optional_env_int("ROLE_WORLD_SIZE"),
        # Parallel role key (e.g. "dp=2,pp=1,tp=0"); surfaces as federation _role.
        "role": current_role() or None,
        "status": "running",
        "timestamp": int(time.time() * 1_000_000),
    }


def _put_node(master_http: str, node: dict[str, Any]) -> None:
    url = f"{master_http.rstrip('/')}/apis/nodes"
    body = json.dumps(node).encode("utf-8")
    req = urllib.request.Request(
        url,
        data=body,
        headers={"Content-Type": "application/json"},
        method="PUT",
    )
    with urllib.request.urlopen(req, timeout=10) as resp:
        if resp.status >= 400:
            raise RuntimeError(f"PUT {url} failed: HTTP {resp.status}")


def publish_master() -> dict[str, str]:
    """Rank 0: bind probing HTTP and publish reachable master info to the job store."""
    import torch.distributed as dist

    if dist.get_rank() != 0:
        raise RuntimeError("publish_master() must be called on rank 0")

    bound = _ensure_http_server()
    reachable = _reachable_addr(bound)
    info = {
        "addr": reachable,
        "http_base": f"http://{reachable}",
        "bound": bound,
    }
    store = _rendezvous_store()
    key = _master_store_key()
    store.set(key, json.dumps(info, separators=(",", ":")))
    logger.info("probing torchrun: published master at %s (key=%s)", reachable, key)
    return info


def wait_master(timeout_sec: float | None = None) -> dict[str, str]:
    """Non-zero ranks: wait for rank 0 master info in the rendezvous store."""
    import torch.distributed as dist

    rank = dist.get_rank()
    if rank == 0:
        bound = _ensure_http_server()
        reachable = _reachable_addr(bound)
        return {
            "addr": reachable,
            "http_base": f"http://{reachable}",
            "bound": bound,
        }

    if timeout_sec is None:
        timeout_sec = float(os.environ.get("PROBING_TORCHRUN_STORE_TIMEOUT", "120"))

    store = _rendezvous_store()
    key = _master_store_key()
    deadline = time.monotonic() + timeout_sec
    while time.monotonic() < deadline:
        try:
            remaining = max(0.1, deadline - time.monotonic())
            store.wait([key], timedelta(seconds=min(1.0, remaining)))
            raw = _store_get_str(store, key)
            info = json.loads(raw)
            if info.get("http_base"):
                logger.info(
                    "probing torchrun: rank %s discovered master %s",
                    rank,
                    info.get("addr"),
                )
                return info
        except Exception as exc:
            logger.debug("waiting for probing master key %s: %s", key, exc)
            time.sleep(0.1)

    raise TimeoutError(
        f"timed out after {timeout_sec}s waiting for probing master at store key {key}"
    )


def register_with_master(master_info: dict[str, str]) -> None:
    """Register this rank's probing HTTP address with the cluster master."""
    local_bound = _ensure_http_server()
    local_addr = _reachable_addr(local_bound)
    master_http = master_info["http_base"]
    node = _node_payload(local_addr)
    try:
        _put_node(master_http, node)
        logger.info(
            "probing torchrun: rank %s registered %s with master %s",
            node.get("rank"),
            local_addr,
            master_info.get("addr"),
        )
    except urllib.error.URLError as exc:
        logger.warning("probing torchrun: failed to register with master: %s", exc)


def setup_torchrun_cluster() -> dict[str, str] | None:
    """
    Full setup: publish or wait for master, then register this rank.

    No-op when disabled, dist is not initialized, or WORLD_SIZE <= 1.
    """
    global _SETUP_DONE
    if not _enabled() or _SETUP_DONE:
        return None

    try:
        import torch.distributed as dist
    except ImportError:
        return None

    if not dist.is_initialized():
        return None

    if dist.get_world_size() <= 1:
        return None

    _SETUP_DONE = True
    rank = dist.get_rank()
    if rank == 0:
        master_info = publish_master()
    else:
        master_info = wait_master()

    global _MASTER_INFO
    _MASTER_INFO = master_info
    register_with_master(master_info)
    return master_info


def refresh_node_role() -> bool:
    """Re-register this node so the master picks up a runtime role change.

    No-op (returns ``False``) when the cluster was never set up. Best-effort:
    swallows network errors so ``set_role`` never raises on registration issues.
    """
    if not _SETUP_DONE or _MASTER_INFO is None:
        return False
    try:
        register_with_master(_MASTER_INFO)
        return True
    except Exception as exc:  # pragma: no cover - network best-effort
        logger.debug("probing torchrun: role refresh failed: %s", exc)
        return False


def master_info() -> dict[str, str] | None:
    """Return cluster master HTTP info after ``setup_torchrun_cluster()``."""
    return _MASTER_INFO


def maybe_setup_torchrun_cluster() -> dict[str, str] | None:
    """Idempotent entry point; safe to call after ``init_process_group``."""
    return setup_torchrun_cluster()

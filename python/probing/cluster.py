"""
Cluster module: discover Pulsing and sync membership to probing server.

Designed for loose coupling:
- Probing manages itself after injection.
- This module discovers Pulsing (global ActorSystem) and, if found, periodically
  pushes system.members() to the probing server's POST /apis/nodes/sync.

Auto 组网 is handled by Pulsing's bootstrap (background thread that runs init_in_ray /
init_in_torchrun). Probing only calls pulsing.bootstrap.wait_ready(timeout) and then
starts the sync loop; no init logic in probing.

Env:
- PROBING_PULSING_DISCOVER: set to "0" to disable (default: enable).
- PROBING_PULSING_BOOTSTRAP_TIMEOUT: seconds to wait for Pulsing cluster (default 30).
- PROBING_SERVER: base URL of probing server (e.g. http://host:port).
  Fallback: PROBING_REPORT_URL, or MASTER_ADDR:PROBING_PORT (PROBING_PORT default 8080).
"""

import json
import logging
import os
import threading
import time
from typing import Any

logger = logging.getLogger(__name__)

_SYNC_THREAD: threading.Thread | None = None
_SYNC_STOP = threading.Event()


def _get_probing_server_url() -> str | None:
    url = os.environ.get("PROBING_SERVER") or os.environ.get("PROBING_REPORT_URL")
    if url:
        return url if url.startswith("http") else f"http://{url}"
    master = os.environ.get("MASTER_ADDR")
    if master:
        port = os.environ.get("PROBING_PORT", "8080")
        return f"http://{master}:{port}"
    return None


def discover_pulsing() -> Any:
    """Return the global Pulsing ActorSystem if available, else None."""
    try:
        import pulsing  # type: ignore[import-untyped]  # optional dependency

        if getattr(pulsing, "is_initialized", lambda: False)():
            return pulsing.get_system()
        return None
    except Exception as e:
        logger.debug("Pulsing not available: %s", e)
        return None


def _members_to_nodes(members: list[dict[str, str]]) -> list[dict[str, Any]]:
    """Map Pulsing members() items to probing Node-like dicts (host, addr, status, timestamp)."""
    import time as t

    ts = int(t.time() * 1_000_000)
    nodes = []
    for m in members:
        addr = m.get("addr") or ""
        # host: use part before ':' if addr is "host:port", else addr
        host = addr.split(":")[0] if ":" in addr else addr or "unknown"
        nodes.append(
            {
                "host": host,
                "addr": addr,
                "local_rank": None,
                "rank": None,
                "world_size": None,
                "group_rank": None,
                "group_world_size": None,
                "role_name": None,
                "role_rank": None,
                "role_world_size": None,
                "status": m.get("status"),
                "timestamp": ts,
            }
        )
    return nodes


def _sync_once(system: Any, server_url: str) -> bool:
    """Fetch members from Pulsing and POST to server. Returns True on success."""
    try:
        # system.members() is async in Pulsing; we run in a sync thread so use asyncio.run
        import asyncio

        members = asyncio.run(_async_members(system))
    except Exception as e:
        logger.debug("Failed to get Pulsing members: %s", e)
        return False
    if not members:
        return True
    nodes = _members_to_nodes(members)
    sync_url = f"{server_url.rstrip('/')}/apis/nodes/sync"
    try:
        import urllib.request

        req = urllib.request.Request(
            sync_url,
            data=json.dumps(nodes).encode("utf-8"),
            headers={"Content-Type": "application/json"},
            method="POST",
        )
        with urllib.request.urlopen(req, timeout=10) as resp:
            if resp.status in (200, 204):
                return True
    except Exception as e:
        logger.debug("Failed to sync nodes to %s: %s", sync_url, e)
    return False


async def _async_members(system: Any):
    """Async wrapper for system.members()."""
    return await system.members()


def _sync_loop(server_url: str, interval_sec: float = 10.0) -> None:
    while not _SYNC_STOP.wait(timeout=interval_sec):
        system = discover_pulsing()
        if system is None:
            continue
        _sync_once(system, server_url)


def start_pulsing_sync(
    server_url: str | None = None, interval_sec: float = 10.0
) -> bool:
    """
    Start a background thread that periodically syncs Pulsing members to the probing server.

    If Pulsing is not yet initialized, calls pulsing.bootstrap(wait_timeout=...) so
    Pulsing's background bootstrap (Ray/torchrun auto 组网) can complete; probing only waits.
    If server_url is None, it is read from PROBING_SERVER / PROBING_REPORT_URL / MASTER_ADDR.
    Returns True if the sync thread was started, False if disabled or no server URL.
    """
    global _SYNC_THREAD
    if os.environ.get("PROBING_PULSING_DISCOVER", "1").strip().lower() in (
        "0",
        "false",
        "no",
    ):
        return False
    url = server_url or _get_probing_server_url()
    if not url:
        logger.debug("No probing server URL; skip Pulsing sync")
        return False
    # Wait for Pulsing cluster (bootstrap runs init_in_ray / init_in_torchrun in background)
    if discover_pulsing() is None:
        try:
            import pulsing  # type: ignore[import-untyped]

            timeout = float(os.environ.get("PROBING_PULSING_BOOTSTRAP_TIMEOUT", "30"))
            if pulsing.bootstrap(wait_timeout=timeout) is not True:
                logger.debug(
                    "Pulsing bootstrap did not become ready within %ss", timeout
                )
        except Exception as e:
            logger.debug("Pulsing bootstrap not available: %s", e)
    if _SYNC_THREAD is not None and _SYNC_THREAD.is_alive():
        return True
    _SYNC_STOP.clear()
    _SYNC_THREAD = threading.Thread(
        target=_sync_loop,
        args=(url, interval_sec),
        name="probing-pulsing-sync",
        daemon=True,
    )
    _SYNC_THREAD.start()
    logger.info("Started Pulsing cluster sync to %s (interval=%ss)", url, interval_sec)
    return True


def stop_pulsing_sync() -> None:
    """Stop the background sync thread."""
    _SYNC_STOP.set()

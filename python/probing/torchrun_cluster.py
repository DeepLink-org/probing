"""Thin Python facade for torchrun cluster registration (Rust implements the worker)."""

from __future__ import annotations

import logging
import os

logger = logging.getLogger(__name__)

_SETUP_DONE = False


def _enabled() -> bool:
    if os.environ.get("PROBING_TORCHRUN_CLUSTER", "1").strip().lower() in (
        "0",
        "false",
        "no",
    ):
        return False
    return True


def _world_size() -> int:
    raw = os.environ.get("WORLD_SIZE", "1").strip()
    try:
        return int(raw)
    except ValueError:
        return 1


def setup_torchrun_cluster() -> dict[str, str] | None:
    """Idempotent: start Rust-side hierarchical cluster heartbeat."""
    global _SETUP_DONE
    if not _enabled() or _SETUP_DONE or _world_size() <= 1:
        return None

    try:
        from probing import _core

        http_base = _core.start_torchrun_cluster()
    except Exception as exc:
        logger.debug("probing torchrun cluster start skipped: %s", exc)
        return None

    _SETUP_DONE = True
    if http_base:
        return {"http_base": http_base, "addr": http_base.removeprefix("http://")}
    return None


def refresh_node_role() -> bool:
    """Re-register after ``probing.set_role`` (syncs ``PROBING_NODE_ROLE`` env)."""
    try:
        from probing.parallel import current_role

        role = current_role()
        if role:
            os.environ["PROBING_NODE_ROLE"] = role
        else:
            os.environ.pop("PROBING_NODE_ROLE", None)
        from probing import _core

        return bool(_core.refresh_torchrun_cluster_role())
    except Exception as exc:
        logger.debug("probing torchrun: role refresh failed: %s", exc)
        return False


def master_info() -> dict[str, str] | None:
    """Return cluster master HTTP info when this process is global rank 0."""
    try:
        from probing import _core

        http_base = _core.start_torchrun_cluster()
    except Exception:
        return None
    if http_base:
        return {"http_base": http_base, "addr": http_base.removeprefix("http://")}
    return None

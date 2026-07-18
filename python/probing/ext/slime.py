"""Slime framework adapter for Probing-RL.

Maps Slime-specific process identity (env vars, Ray actor cmdlines) onto
framework-neutral agentic-RL roles consumed by the Web UI and ``python.ray_process``.

Generic code must not import Slime class names; call into this module instead.
"""

from __future__ import annotations

import os
from typing import Optional, Tuple

# (cmdline needle, neutral role). Order matters: first match wins.
# Needles are Slime / SGLang actor class names and entrypoints only.
_CMDLINE_MARKERS: Tuple[Tuple[str, str], ...] = (
    ("RolloutManager.generate", "rollout_actor"),
    ("MegatronTrainRayActor.train", "train_actor"),
    ("SGLangEngine", "inference_engine"),
    ("sglang.launch_server", "inference_engine"),
    ("train_async.py", "driver"),
)


def apply_env_bridge() -> None:
    """Copy Slime env vars into generic Probing identity keys.

    Safe to call multiple times. Does not overwrite explicit ``PROBING_*`` values.
    After this runs, callers should only read ``PROBING_*`` / ``POD_IP`` / ``RAY_NODE_IP``.
    """

    if not os.environ.get("PROBING_RAY_PROCESS_ROLE") and os.environ.get("SLIME_PROBING_ROLE"):
        os.environ["PROBING_RAY_PROCESS_ROLE"] = os.environ["SLIME_PROBING_ROLE"]

    if not os.environ.get("PROBING_NODE_IP") and os.environ.get("SLIME_NODE_IP"):
        os.environ["PROBING_NODE_IP"] = os.environ["SLIME_NODE_IP"]


def infer_role_from_cmdline(cmd: str) -> Optional[str]:
    """Infer a neutral process role from a Slime/Ray actor cmdline."""

    if not cmd:
        return None
    for needle, role in _CMDLINE_MARKERS:
        if needle in cmd:
            return role
    return None


def resolve_process_role(*, cmdline: Optional[str] = None, default: str = "") -> str:
    """Resolve process role: bridge Slime env → read ``PROBING_*`` → cmdline → default."""

    apply_env_bridge()
    role = (
        os.environ.get("PROBING_RAY_PROCESS_ROLE", "").strip()
        or os.environ.get("PROBING_PROCESS_ROLE", "").strip()
    )
    if role:
        return role
    if cmdline:
        inferred = infer_role_from_cmdline(cmdline)
        if inferred:
            return inferred
    return default


def resolve_node_ip() -> str:
    """Resolve node IP after bridging Slime → generic keys."""

    apply_env_bridge()
    return (
        os.environ.get("PROBING_NODE_IP", "").strip()
        or os.environ.get("POD_IP", "").strip()
        or os.environ.get("RAY_NODE_IP", "").strip()
    )

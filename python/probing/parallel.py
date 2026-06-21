"""3D parallel topology coordinates for distributed training spans.

Reads Megatron-style and generic environment variables so every span and
collective row can be correlated with (TP, PP, DP) placement — the same
coordinate system MegaScale uses for hang diagnosis (§5.2).
"""

from __future__ import annotations

import os
from dataclasses import dataclass
from typing import Dict, Optional


def _read_int(keys: tuple[str, ...]) -> Optional[int]:
    for key in keys:
        raw = os.environ.get(key)
        if raw is None or not str(raw).strip():
            continue
        try:
            return int(str(raw).strip())
        except ValueError:
            continue
    return None


@dataclass(frozen=True)
class ParallelTopology:
    tp_rank: int = -1
    pp_rank: int = -1
    dp_rank: int = -1
    tp_size: int = -1
    pp_size: int = -1
    dp_size: int = -1

    def as_dict(self) -> Dict[str, int]:
        return {
            "tp_rank": self.tp_rank,
            "pp_rank": self.pp_rank,
            "dp_rank": self.dp_rank,
            "tp_size": self.tp_size,
            "pp_size": self.pp_size,
            "dp_size": self.dp_size,
        }


def _or_sentinel(value: Optional[int]) -> int:
    return value if value is not None else -1


def parallel_topology() -> ParallelTopology:
    """Snapshot parallel ranks/sizes from the environment."""
    return ParallelTopology(
        tp_rank=_or_sentinel(
            _read_int(("TENSOR_MODEL_PARALLEL_RANK", "TP_RANK", "PROBING_TP_RANK"))
        ),
        pp_rank=_or_sentinel(
            _read_int(("PIPELINE_MODEL_PARALLEL_RANK", "PP_RANK", "PROBING_PP_RANK"))
        ),
        dp_rank=_or_sentinel(
            _read_int(("DATA_PARALLEL_RANK", "DP_RANK", "PROBING_DP_RANK"))
        ),
        tp_size=_or_sentinel(
            _read_int(("TENSOR_MODEL_PARALLEL_SIZE", "TP_SIZE", "PROBING_TP_SIZE"))
        ),
        pp_size=_or_sentinel(
            _read_int(("PIPELINE_MODEL_PARALLEL_SIZE", "PP_SIZE", "PROBING_PP_SIZE"))
        ),
        dp_size=_or_sentinel(
            _read_int(("DATA_PARALLEL_SIZE", "DP_SIZE", "PROBING_DP_SIZE"))
        ),
    )


def parallel_fields() -> Dict[str, int]:
    """Topology dict omitting unset (-1) entries."""
    return {k: v for k, v in parallel_topology().as_dict().items() if v >= 0}


# Parallel-role dimensions: canonical short name -> candidate env vars (priority
# order). Register a new dimension here (e.g. expert/context parallel) and it is
# picked up everywhere — no table schema change, because the role is encoded as a
# single string column rather than one column per dimension.
_ROLE_DIMENSIONS = (
    ("tp", ("TENSOR_MODEL_PARALLEL_RANK", "TP_RANK", "PROBING_TP_RANK")),
    ("pp", ("PIPELINE_MODEL_PARALLEL_RANK", "PP_RANK", "PROBING_PP_RANK")),
    ("dp", ("DATA_PARALLEL_RANK", "DP_RANK", "PROBING_DP_RANK")),
    (
        "ep",
        (
            "EXPERT_MODEL_PARALLEL_RANK",
            "EXPERT_PARALLEL_RANK",
            "EP_RANK",
            "PROBING_EP_RANK",
        ),
    ),
    ("cp", ("CONTEXT_PARALLEL_RANK", "CP_RANK", "PROBING_CP_RANK")),
)

# Ad-hoc dimensions: ``PROBING_ROLE_<NAME>=<int>`` adds ``<name>`` with no code change.
_ROLE_ENV_PREFIX = "PROBING_ROLE_"


def role_dims() -> Dict[str, int]:
    """Collect all set parallel-role dimensions as ``{name: rank}``.

    Known dimensions come from ``_ROLE_DIMENSIONS``; any ``PROBING_ROLE_<NAME>``
    environment variable contributes a custom dimension.
    """
    dims: Dict[str, int] = {}
    for name, keys in _ROLE_DIMENSIONS:
        value = _read_int(keys)
        if value is not None and value >= 0:
            dims[name] = value
    for key, raw in os.environ.items():
        if not key.startswith(_ROLE_ENV_PREFIX):
            continue
        name = key[len(_ROLE_ENV_PREFIX) :].strip().lower()
        if not name or name in dims:
            continue
        try:
            value = int(str(raw).strip())
        except (TypeError, ValueError):
            continue
        if value >= 0:
            dims[name] = value
    return dims


def role_key() -> str:
    """Encode the parallel role as a canonical, sortable string.

    Example: ``"dp=2,pp=1,tp=0"`` — dimensions are sorted by name so the same
    physical role always yields the same key (suitable for ``GROUP BY role``).
    Returns ``""`` when no parallel-role env is set. Prefer this single field
    over per-dimension columns so new dimensions need no schema change.
    """
    dims = role_dims()
    return ",".join(f"{name}={dims[name]}" for name in sorted(dims))


# --- Runtime role override -------------------------------------------------
# The effective role is normally derived from env (role_key). ``set_role`` lets
# a process override it at runtime (e.g. frameworks that decide placement after
# import). ``current_role`` is the effective value used by data rows and the
# cluster Node payload; prefer it over ``role_key`` at call sites.
_ROLE_OVERRIDE: Optional[str] = None


def _normalize_role(role) -> str:
    """Normalize a role spec (str | mapping | None) to a canonical KV string."""
    if role is None:
        return ""
    if isinstance(role, str):
        return role.strip()
    if isinstance(role, dict):
        parts = []
        for name in sorted(role):
            value = role[name]
            if value is None:
                continue
            parts.append(f"{str(name).strip()}={value}")
        return ",".join(parts)
    raise TypeError(f"role must be str, dict, or None, got {type(role).__name__}")


def current_role() -> str:
    """Effective parallel role for this process: runtime override if set,
    otherwise derived from environment via ``role_key``."""
    if _ROLE_OVERRIDE is not None:
        return _ROLE_OVERRIDE
    return role_key()


def set_role(role=None, **dims) -> str:
    """Override this process's parallel role at runtime.

    Accepts a canonical string, a mapping, or keyword dimensions::

        set_role("dp=2,pp=1,tp=0")
        set_role({"dp": 2, "pp": 1})
        set_role(dp=2, pp=1, tp=0)

    Returns the normalized role string now in effect. New ``torch_trace`` /
    ``comm_collective`` rows pick it up immediately; the cluster Node entry is
    refreshed best-effort so federation ``_role`` tags stay consistent.
    """
    global _ROLE_OVERRIDE
    spec = dict(dims) if dims else role
    _ROLE_OVERRIDE = _normalize_role(spec)
    _refresh_cluster_role()
    return _ROLE_OVERRIDE


def clear_role() -> None:
    """Drop the runtime override; revert to the env-derived role."""
    global _ROLE_OVERRIDE
    _ROLE_OVERRIDE = None


def _refresh_cluster_role() -> None:
    """Best-effort re-register this node so the master sees the new role."""
    try:
        from probing import torchrun_cluster

        torchrun_cluster.refresh_node_role()
    except Exception:  # pragma: no cover - cluster may be absent/not set up
        pass

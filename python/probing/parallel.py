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

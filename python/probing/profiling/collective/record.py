"""Persisted collective communication rows (query as ``python.comm_collective``).

``lite`` mode (default): one ``comm_collective`` row + closed ``trace_event`` pair
(timing + context, no span stack / ``inspect.stack``).

``full`` mode: live ``comm.*`` spans on the stack (for nesting during the call).
"""

from __future__ import annotations

import json
from dataclasses import dataclass
from enum import Enum
from typing import Iterable, Optional

from probing.core import table
from probing.parallel import current_role
from probing.tracing import (
    _step_fields,
    comm_kind,
    record_closed_span,
    recorded_span,
    step_snapshot,
)


class CommRecordMode(str, Enum):
    LITE = "lite"
    FULL = "full"


@table("comm_collective")
@dataclass
class CommCollective:
    local_step: int = 0
    global_step: int = 0
    rank: int = -1
    world_size: int = -1
    # Extensible parallel role (e.g. "dp=2,pp=1,tp=0"); see probing.parallel.role_key.
    role: str = ""
    op: str = ""
    group_rank: int = 0
    group_size: int = 0
    participate_ranks: str = ""
    tensor_shape: str = ""
    tensor_dtype: str = ""
    bytes: int = 0
    duration_ms: float = 0.0
    async_op: int = 0


def _role_row_fields() -> dict:
    return {"role": current_role()}


def _step_row_fields() -> dict:
    snap = step_snapshot()
    if snap is None:
        return {"local_step": 0, "global_step": 0, "rank": -1, "world_size": -1}
    step = _step_fields(snap)
    return {
        "local_step": step.get("local_step", 0),
        "global_step": step.get("global_step", 0),
        "rank": step.get("rank", -1),
        "world_size": step.get("world_size", -1),
    }


def _context_fields(
    *,
    op: str,
    group_rank: int,
    group_size: int,
    participate_ranks: Iterable[int],
    tensor_shape: str = "",
    tensor_dtype: str = "",
    nbytes: int = 0,
    async_op: bool = False,
) -> dict:
    ranks_json = json.dumps(list(participate_ranks)) if participate_ranks else ""
    return {
        **_step_row_fields(),
        **_role_row_fields(),
        "op": op,
        "group_rank": group_rank,
        "group_size": group_size,
        "participate_ranks": ranks_json,
        "tensor_shape": tensor_shape,
        "tensor_dtype": tensor_dtype,
        "bytes": nbytes,
        "async_op": int(async_op),
    }


def record_comm_lite(
    *,
    op: str,
    duration_ms: float,
    group_rank: int,
    group_size: int,
    participate_ranks: Optional[Iterable[int]] = None,
    tensor_shape: str = "",
    tensor_dtype: str = "",
    nbytes: int = 0,
    async_op: bool = False,
    write_trace_event: bool = True,
) -> None:
    """Append timing + context; optionally mirror to ``python.trace_event``."""
    fields = _context_fields(
        op=op,
        group_rank=group_rank,
        group_size=group_size,
        participate_ranks=participate_ranks or (),
        tensor_shape=tensor_shape,
        tensor_dtype=tensor_dtype,
        nbytes=nbytes,
        async_op=async_op,
    )
    CommCollective(duration_ms=duration_ms, **fields).save()
    if write_trace_event:
        record_closed_span(
            op,
            kind=comm_kind(op),
            duration_ns=int(duration_ms * 1e6),
            attrs={**fields, "duration_ms": duration_ms},
            source="collective_tracer",
        )


def begin_comm_span(
    op: str,
    *,
    group_rank: int,
    group_size: int,
    participate_ranks: Iterable[int],
    tensor_shape: str,
    tensor_dtype: str,
    nbytes: int,
    async_op: bool = False,
):
    """Enter a ``comm.*`` span (``full`` mode only)."""
    meta = _context_fields(
        op=op,
        group_rank=group_rank,
        group_size=group_size,
        participate_ranks=participate_ranks,
        tensor_shape=tensor_shape,
        tensor_dtype=tensor_dtype,
        nbytes=nbytes,
        async_op=async_op,
    )
    span_attrs = {**meta, "source": "collective_tracer"}
    cm = recorded_span(op, kind=comm_kind(op), **span_attrs)
    cm.__enter__()
    return cm, meta


def finish_comm_span(
    cm,
    meta: dict,
    *,
    op: str,
    duration_ms: float,
    group_rank: int,
    group_size: int,
) -> None:
    """Close span and append ``python.comm_collective`` row (``full`` mode)."""
    if cm is not None:
        cm.__exit__(None, None, None)

    row = {**meta, "op": op, "group_rank": group_rank, "group_size": group_size}
    CommCollective(duration_ms=duration_ms, **row).save()

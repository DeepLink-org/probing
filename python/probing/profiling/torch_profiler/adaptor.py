"""Compile torch.profiler output into profile_capture + profile_hotspot rows."""

from __future__ import annotations

import logging
import os
import time
import uuid
from dataclasses import dataclass
from typing import Any, Optional

from probing.parallel import current_role
from probing.tracing.coordinates import row_fields
from probing.tracing import step

from .session_store import CaptureRecord, HotspotRecord

logger = logging.getLogger(__name__)


def _max_events() -> int:
    raw = os.environ.get("PROBING_TORCH_PROFILER_MAX_EVENTS", "200000").strip()
    try:
        value = int(raw)
    except ValueError:
        return 200000
    return max(value, 1000)


@dataclass
class _BucketAgg:
    bucket_kind: str
    bucket_name: str
    self_us: int = 0
    wall_us: int = 0
    calls: int = 0


def _bucket_kind_for_name(name: str) -> str:
    lower = name.lower()
    if "nccl" in lower:
        return "collective"
    if "memcpy" in lower or lower.startswith("memcpy"):
        return "memcpy"
    if "cudadevicesynchronize" in lower or "cudastreamsynchronize" in lower:
        return "cuda_runtime"
    if lower.startswith("cuda") and (
        "launch" in lower or "malloc" in lower or "free" in lower or "sync" in lower
    ):
        return "cuda_runtime"
    if lower.startswith("aten::") or lower.startswith("autograd::"):
        return "cpu_op"
    return "kernel"


def _event_name(event: Any) -> str:
    for attr in ("key", "name"):
        value = getattr(event, attr, None)
        if value:
            return str(value)
    return "unknown"


def _event_self_us(event: Any) -> int:
    cuda = int(getattr(event, "self_cuda_time_total", 0) or 0)
    cpu = int(getattr(event, "self_cpu_time_total", 0) or 0)
    return cuda if cuda > 0 else cpu


def _event_wall_us(event: Any) -> int:
    cuda = int(getattr(event, "cuda_time_total", 0) or 0)
    cpu = int(getattr(event, "cpu_time_total", 0) or 0)
    return cuda if cuda > 0 else cpu


def _event_calls(event: Any) -> int:
    return max(int(getattr(event, "count", 0) or 0), 1)


def compile_key_averages(
    events: list[Any],
    *,
    trigger: str,
    steps_profiled: int,
    started_at_us: int,
    ended_at_us: int,
    capture_id: Optional[str] = None,
    status: str = "completed",
    error: str = "",
) -> tuple[CaptureRecord, list[HotspotRecord]]:
    """Build capture + hotspot rows from profiler key_averages() events."""
    coords = row_fields(step.snapshot())
    rank = int(coords.get("rank", -1))
    local_step = int(coords.get("local_step", -1))
    global_step = int(coords.get("global_step", -1))

    truncated = False
    original_event_count = len(events)
    max_events = _max_events()
    if original_event_count > max_events:
        truncated = True
        events = events[:max_events]

    aggs: dict[tuple[str, str], _BucketAgg] = {}
    for event in events:
        name = _event_name(event)
        kind = _bucket_kind_for_name(name)
        key = (kind, name)
        agg = aggs.get(key)
        if agg is None:
            agg = _BucketAgg(bucket_kind=kind, bucket_name=name)
            aggs[key] = agg
        agg.self_us += _event_self_us(event)
        agg.wall_us += _event_wall_us(event)
        agg.calls += _event_calls(event)

    total_self_us = sum(a.self_us for a in aggs.values())
    wall_us = max(ended_at_us - started_at_us, 0)
    if wall_us <= 0:
        wall_us = total_self_us

    capture = CaptureRecord(
        capture_id=capture_id or uuid.uuid4().hex,
        local_step=local_step,
        global_step=global_step,
        rank=rank,
        world_size=int(coords.get("world_size", -1)),
        role=current_role(),
        trigger=trigger,
        steps_profiled=steps_profiled,
        wall_us=wall_us,
        started_at_us=started_at_us,
        ended_at_us=ended_at_us,
        status=status,
        truncated=truncated,
        event_count=original_event_count,
        error=error,
    )

    hotspots: list[HotspotRecord] = []
    denom = wall_us if wall_us > 0 else max(total_self_us, 1)
    for agg in sorted(aggs.values(), key=lambda a: a.self_us, reverse=True):
        hotspots.append(
            HotspotRecord(
                capture_id=capture.capture_id,
                local_step=local_step,
                global_step=global_step,
                rank=rank,
                bucket_kind=agg.bucket_kind,
                bucket_name=agg.bucket_name,
                self_us=agg.self_us,
                wall_us=agg.wall_us,
                calls=agg.calls,
                pct_of_capture=agg.self_us / denom,
            )
        )
    return capture, hotspots


def compile_from_profiler(
    profiler: Any,
    *,
    trigger: str,
    steps_profiled: int,
    started_at_us: int,
    ended_at_us: Optional[int] = None,
    capture_id: Optional[str] = None,
    status: str = "completed",
    error: str = "",
) -> tuple[CaptureRecord, list[HotspotRecord]]:
    """Compile a finished torch.profiler profile into SQL rows."""
    end_us = ended_at_us if ended_at_us is not None else _now_us()
    events: list[Any] = []
    out_status = status
    out_error = error
    try:
        averages = profiler.key_averages()
        events = list(averages) if averages is not None else []
    except Exception as exc:
        logger.debug("profiler.key_averages failed: %s", exc)
        out_error = out_error or str(exc)

    if not events:
        try:
            raw = profiler.events()
            events = list(raw) if raw is not None else []
        except Exception as exc:
            logger.debug("profiler.events fallback failed: %s", exc)
            out_error = out_error or str(exc)

    if not events and out_error and out_status == "completed":
        out_status = "failed"
    elif events and out_status == "completed":
        out_error = ""

    return compile_key_averages(
        events,
        trigger=trigger,
        steps_profiled=steps_profiled,
        started_at_us=started_at_us,
        ended_at_us=end_us,
        capture_id=capture_id,
        status=out_status,
        error=out_error,
    )


def _now_us() -> int:
    return int(time.time() * 1_000_000)

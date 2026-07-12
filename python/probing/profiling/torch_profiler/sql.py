"""SQL row providers for python.profile_capture and python.profile_hotspot."""

from __future__ import annotations

from typing import Any

import probing

from .session_store import CaptureRecord, HotspotRecord, get_session_store

_CAPTURE_DOC = (
    "On-demand torch.profiler capture anchor (step, rank, quality metadata). "
    "One row per profile window."
)
_HOTSPOT_DOC = (
    "Conclusion fact table: aggregated kernel/op time buckets per capture. "
    "Fed by KinetoSqlAdaptor at finalize."
)

_COLUMN_DOCS: dict[str, dict[str, str]] = {
    "profile_capture": {
        "capture_id": "Unique capture id (join key)",
        "local_step": "Per-rank training step at finalize",
        "global_step": "Global training step at finalize",
        "rank": "torch.distributed rank (-1 unknown)",
        "world_size": "World size (-1 unknown)",
        "role": "Parallel role key (dp=…,pp=…)",
        "trigger": "Who started capture (manual, skill, http)",
        "steps_profiled": "Profiler window length in optimizer steps",
        "wall_us": "Capture wall time (microseconds); Q2 denominator",
        "started_at_us": "Capture start (epoch microseconds)",
        "ended_at_us": "Capture end (epoch microseconds)",
        "status": "running | completed | failed",
        "truncated": "1 if event list was truncated",
        "event_count": "Raw profiler event count before aggregation",
        "error": "Failure message when status=failed",
    },
    "profile_hotspot": {
        "capture_id": "FK to profile_capture",
        "local_step": "Training step (query without capture_id)",
        "global_step": "Global step",
        "rank": "Rank that produced this row",
        "bucket_kind": "kernel | cpu_op | memcpy | cuda_runtime | collective | other",
        "bucket_name": "Kernel or op name",
        "self_us": "Self time microseconds (primary sort key)",
        "wall_us": "Wall/subtree time microseconds",
        "calls": "Invocation count in capture",
        "pct_of_capture": "self_us / capture.wall_us",
        "module_hint": "Module hint from stack (v2)",
    },
}

_DOCS_REGISTERED = False


def _register_docs_once() -> None:
    global _DOCS_REGISTERED
    if _DOCS_REGISTERED:
        return
    probing.register_table_docs(
        "python.profile_capture", _CAPTURE_DOC, _COLUMN_DOCS["profile_capture"]
    )
    probing.register_table_docs(
        "python.profile_hotspot", _HOTSPOT_DOC, _COLUMN_DOCS["profile_hotspot"]
    )
    _DOCS_REGISTERED = True


def _capture_to_dict(row: CaptureRecord) -> dict[str, Any]:
    return {
        "capture_id": row.capture_id,
        "local_step": row.local_step,
        "global_step": row.global_step,
        "rank": row.rank,
        "world_size": row.world_size,
        "role": row.role,
        "trigger": row.trigger,
        "steps_profiled": row.steps_profiled,
        "wall_us": row.wall_us,
        "started_at_us": row.started_at_us,
        "ended_at_us": row.ended_at_us,
        "status": row.status,
        "truncated": 1 if row.truncated else 0,
        "event_count": row.event_count,
        "error": row.error,
    }


def _hotspot_to_dict(row: HotspotRecord) -> dict[str, Any]:
    return {
        "capture_id": row.capture_id,
        "local_step": row.local_step,
        "global_step": row.global_step,
        "rank": row.rank,
        "bucket_kind": row.bucket_kind,
        "bucket_name": row.bucket_name,
        "self_us": row.self_us,
        "wall_us": row.wall_us,
        "calls": row.calls,
        "pct_of_capture": row.pct_of_capture,
        "module_hint": row.module_hint,
    }


def profile_capture_rows() -> list[dict[str, Any]]:
    """Rows for ``SELECT * FROM python.profile_capture``."""
    _register_docs_once()
    return [_capture_to_dict(c) for c in get_session_store().captures()]


def profile_hotspot_rows() -> list[dict[str, Any]]:
    """Rows for ``SELECT * FROM python.profile_hotspot``."""
    _register_docs_once()
    return [_hotspot_to_dict(h) for h in get_session_store().hotspots()]

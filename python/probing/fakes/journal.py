"""Ground-truth journal for ``probing.fakes`` (query as ``python.fake_event``).

Fake Megatron / Torch stubs append rows here so they can be JOINed against
probing observability tables (``python.trace_event``, step coords, optional
``python.comm_collective``) to check that instrumentation saw what the fake
layer intended.
"""

from __future__ import annotations

import json
import logging
import threading
import time
import uuid
from dataclasses import dataclass
from typing import Any, Optional

from probing.core import table
from probing.parallel import current_role
from probing.tracing import step
from probing.tracing.coordinates import row_fields

logger = logging.getLogger(__name__)

_SEQ = 0
_SEQ_LOCK = threading.Lock()
_ENABLED = True
_RUN_ID = ""


def set_journal_enabled(enabled: bool) -> None:
    global _ENABLED
    _ENABLED = bool(enabled)


def journal_enabled() -> bool:
    return _ENABLED


def current_run_id() -> str:
    return _RUN_ID


def begin_run(run_id: Optional[str] = None) -> str:
    """Start a new journal run (resets seq, sets ``run_id``)."""
    global _RUN_ID
    reset_journal_seq()
    _RUN_ID = (run_id or uuid.uuid4().hex[:12]).strip()
    return _RUN_ID


def reset_journal_seq() -> None:
    global _SEQ
    with _SEQ_LOCK:
        _SEQ = 0


def _next_seq() -> int:
    global _SEQ
    with _SEQ_LOCK:
        _SEQ += 1
        return _SEQ


@table("fake_event")
@dataclass
class FakeEvent:
    """One intentional fake-layer action (ground truth for correlation)."""

    seq: int = 0
    run_id: str = ""
    kind: str = ""
    name: str = ""
    expected_iteration: int = -1
    expected_micro_batches: int = 1
    expected_role: str = ""
    device: str = ""
    nbytes: int = 0
    duration_ms: float = 0.0
    attrs: str = ""
    micro_step: int = 0
    local_step: int = 0
    global_step: int = 0
    micro_batches: int = 1
    rank: int = -1
    world_size: int = -1
    role: str = ""


def record_fake_event(
    kind: str,
    name: str = "",
    *,
    expected_iteration: Optional[int] = None,
    expected_micro_batches: Optional[int] = None,
    expected_role: Optional[str] = None,
    device: str = "",
    nbytes: int = 0,
    duration_ms: float = 0.0,
    attrs: Optional[dict[str, Any]] = None,
) -> Optional[FakeEvent]:
    """Append a ``python.fake_event`` row and mirror a short log line."""
    if not _ENABLED:
        return None

    from probing.fakes.specs import megatron as megatron_spec

    state = megatron_spec.get_state()
    exp_iter = (
        int(state["iteration"])
        if expected_iteration is None
        else int(expected_iteration)
    )
    exp_mb = (
        int(state["micro_batches"])
        if expected_micro_batches is None
        else int(expected_micro_batches)
    )
    if expected_role is None:
        try:
            expected_role = current_role() or ""
        except Exception:
            expected_role = ""

    coords = row_fields(step.snapshot())
    try:
        role = current_role() or ""
    except Exception:
        role = ""

    if not device:
        try:
            from probing.fakes.device import target_device

            device = target_device()
        except Exception:
            device = ""

    seq = _next_seq()
    row = FakeEvent(
        seq=seq,
        run_id=_RUN_ID or "none",
        kind=str(kind),
        name=str(name or kind),
        expected_iteration=exp_iter,
        expected_micro_batches=exp_mb,
        expected_role=str(expected_role or ""),
        device=str(device),
        nbytes=int(nbytes),
        duration_ms=float(duration_ms),
        attrs=json.dumps(attrs or {}, sort_keys=True),
        micro_step=int(coords.get("micro_step", 0)),
        local_step=int(coords.get("local_step", 0)),
        global_step=int(coords.get("global_step", 0)),
        micro_batches=int(coords.get("micro_batches", 1)),
        rank=int(coords.get("rank", -1)),
        world_size=int(coords.get("world_size", -1)),
        role=role,
    )
    try:
        row.save()
    except Exception as exc:
        logger.debug("fake_event save skipped: %s", exc)
        return row

    logger.info(
        "fake_event run=%s seq=%s kind=%s name=%s expected_iter=%s local_step=%s",
        row.run_id,
        seq,
        kind,
        name or kind,
        exp_iter,
        row.local_step,
    )
    return row


def timed_fake_event(kind: str, name: str = "", **kwargs: Any):
    """Context manager that records duration_ms on exit."""

    class _Timer:
        def __init__(self) -> None:
            self.t0 = 0.0
            self.event: Optional[FakeEvent] = None

        def __enter__(self) -> _Timer:
            self.t0 = time.perf_counter()
            return self

        def __exit__(self, *exc: Any) -> None:
            dt = (time.perf_counter() - self.t0) * 1e3
            self.event = record_fake_event(kind, name, duration_ms=dt, **kwargs)

    return _Timer()

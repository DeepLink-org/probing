"""Training step coordinates and span/table context fields."""

from __future__ import annotations

from typing import Any, Optional

from probing.tracing._bindings import (
    advance_micro_step,
    set_micro_batches,
    step_snapshot,
    sync_micro_step,
)

_ROW_DEFAULTS = {
    "micro_step": 0,
    "local_step": 0,
    "global_step": 0,
    "micro_batches": 1,
    "rank": -1,
    "world_size": -1,
}


class Step:
    """Training step controller.

    * ``micro_step`` — finest counter; ``probing.step()`` advances by one.
    * ``local_step = micro_step // micro_batches`` — per-rank training step.
    * ``global_step = local_step``.
    """

    def __call__(
        self, value: Optional[int] = None, *, micro_batches: Optional[int] = None
    ) -> None:
        if micro_batches is not None:
            set_micro_batches(micro_batches)
        if value is not None:
            sync_micro_step(value)
            return
        if micro_batches is not None:
            return
        advance_micro_step()

    @property
    def micro_step(self) -> int:
        return int(step_snapshot().micro_step)

    @property
    def local_step(self) -> int:
        return int(step_snapshot().local_step)

    @property
    def global_step(self) -> int:
        return int(step_snapshot().global_step)

    def snapshot(self) -> Any:
        return step_snapshot()


step = Step()


def step_fields(snapshot) -> dict:
    """Step/topology fields from a snapshot."""
    if snapshot is None:
        return {}
    local = int(snapshot.local_step)
    return {
        "micro_step": int(snapshot.micro_step),
        "local_step": local,
        "global_step": int(snapshot.global_step),
        "micro_batches": int(snapshot.micro_batches),
        "rank": int(snapshot.rank),
        "world_size": int(snapshot.world_size),
    }


def row_fields(snapshot=None) -> dict:
    """Step coordinates with memtable-friendly defaults."""
    snap = snapshot if snapshot is not None else step.snapshot()
    fields = step_fields(snap)
    return {key: fields.get(key, default) for key, default in _ROW_DEFAULTS.items()}


def span_attrs(user: dict, *, source: str = "manual") -> dict:
    """Merge user attrs with step coordinates, topology, and source label."""
    merged = dict(user)
    merged.setdefault("source", source)
    merged.update(step_fields(step.snapshot()))
    from probing.parallel import parallel_fields

    merged.update(parallel_fields())
    return merged

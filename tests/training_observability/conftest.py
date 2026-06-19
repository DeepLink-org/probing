"""Shared fixtures for training observability (collective, step spans, topology).

Test pyramid for this package
-----------------------------
* **Policy** — autostart / config (`test_collective_policy.py`)
* **Recording** — memtable rows + trace_event (`test_collective_recording.py`)
* **Context** — TP/PP/DP injection (`test_topology_context.py`)
* **SQL / analytics** — same queries the server Training API uses (`test_step_straggler_sql.py`)
* **Iteration E2E** — one synthetic training step end-to-end (`test_training_iteration_e2e.py`)
* **Hooks** — CollectiveTracer with mocked torch.distributed (`test_collective_tracer_hook.py`)

Run::

    PROBING=1 pytest tests/training_observability -q
"""

from __future__ import annotations

import dataclasses
import json
from typing import Any, Iterable, Type

import pytest

# Same shape as probing/server/src/server/training.rs STEP_MATRIX_SQL (local window).
STEP_MATRIX_SQL = """
SELECT
    s.attributes,
    s.name,
    s.time AS start_time,
    CAST((e.time - s.time) / 1000 AS DOUBLE) AS duration_us
FROM python.trace_event s
JOIN python.trace_event e
  ON s.span_id = e.span_id AND e.record_type = 'span_end'
WHERE s.record_type = 'span_start' AND s.kind = 'train.step'
ORDER BY s.time ASC
"""

COMM_COLLECTIVE_RECENT_SQL = """
SELECT local_step, rank, op, group_size, duration_ms, bytes,
       tp_rank, pp_rank, dp_rank, tp_size, pp_size, dp_size
FROM python.comm_collective
ORDER BY timestamp DESC
"""


@pytest.fixture(autouse=True)
def _reset_step_coordinates():
    from probing.tracing import set_step_bucket_size, sync_local_step

    sync_local_step(0)
    set_step_bucket_size(1)
    yield
    sync_local_step(0)
    set_step_bucket_size(1)


@pytest.fixture(autouse=True)
def _reset_observability_tables():
    from probing.profiling.collective.record import CommCollective
    from probing.tracing import TraceEvent

    for table_cls in (CommCollective, TraceEvent):
        try:
            table_cls.drop()
        except Exception:
            pass
        try:
            table_cls.init_table()
        except Exception:
            pass
    yield


@pytest.fixture
def rank_env(monkeypatch):
    """Simulate distributed rank/world_size for span and comm row attributes."""

    def _apply(
        *, rank: int, world_size: int = 8, local_step: int | None = None
    ) -> None:
        from unittest.mock import MagicMock

        from probing.tracing import step_snapshot as real_step_snapshot

        def _fake_snapshot():
            base = real_step_snapshot()
            snap = MagicMock()
            snap.local_step = local_step if local_step is not None else base.local_step
            snap.global_step = base.global_step
            snap.bucket_size = base.bucket_size
            snap.rank = rank
            snap.world_size = world_size
            return snap

        monkeypatch.setenv("RANK", str(rank))
        monkeypatch.setenv("WORLD_SIZE", str(world_size))
        for target in (
            "probing.tracing.step_snapshot",
            "probing.profiling.collective.record.step_snapshot",
        ):
            monkeypatch.setattr(target, _fake_snapshot)

    return _apply


@pytest.fixture
def parallel_env(monkeypatch):
    """Set Megatron-style TP/PP/DP env vars."""

    def _apply(
        *,
        tp_rank: int = 0,
        pp_rank: int = 0,
        dp_rank: int = 0,
        tp_size: int = 2,
        pp_size: int = 2,
        dp_size: int = 4,
    ) -> None:
        monkeypatch.setenv("TENSOR_MODEL_PARALLEL_RANK", str(tp_rank))
        monkeypatch.setenv("PIPELINE_MODEL_PARALLEL_RANK", str(pp_rank))
        monkeypatch.setenv("DATA_PARALLEL_RANK", str(dp_rank))
        monkeypatch.setenv("TENSOR_MODEL_PARALLEL_SIZE", str(tp_size))
        monkeypatch.setenv("PIPELINE_MODEL_PARALLEL_SIZE", str(pp_size))
        monkeypatch.setenv("DATA_PARALLEL_SIZE", str(dp_size))

    return _apply


@pytest.fixture
def sql_query():
    """Run SQL against the in-process engine (requires PROBING=1)."""
    from probing import query

    def _run(expr: str, *, limit: int | None = None):
        sql = expr.strip()
        if limit is not None:
            sql = f"{sql} LIMIT {limit}"
        return query(sql)

    return _run


def table_rows(table_cls: Type[Any], n: int = 50) -> list[dict[str, Any]]:
    """Materialize recent memtable rows as dicts."""
    raw = table_cls.take(n)
    fields = [f.name for f in dataclasses.fields(table_cls)]
    return [dict(zip(fields, data)) for _ts, data in raw]


def train_step_samples_from_memtable(limit: int = 500) -> list[dict[str, Any]]:
    """Mirror ``STEP_MATRIX_SQL`` using memtable rows (agent-side E2E).

    The SQL engine path for ``python.trace_event`` is not stable in all dev
    environments; this helper validates the same join semantics in-process.
    """
    from probing.tracing import TraceEvent

    events = table_rows(TraceEvent, limit)
    starts = {
        e["span_id"]: e
        for e in events
        if e.get("record_type") == "span_start" and e.get("kind") == "train.step"
    }
    ends = {e["span_id"]: e for e in events if e.get("record_type") == "span_end"}
    out: list[dict[str, Any]] = []
    for span_id, start in starts.items():
        end = ends.get(span_id)
        if end is None:
            continue
        attrs_raw = start.get("attributes") or "{}"
        meta = json.loads(attrs_raw) if isinstance(attrs_raw, str) else dict(attrs_raw)
        start_us = int(start.get("time", 0)) // 1000
        end_us = int(end.get("time", 0)) // 1000
        out.append(
            {
                "rank": int(meta.get("rank", -1)),
                "local_step": int(meta.get("local_step", -1)),
                "duration_ms": (end_us - start_us) / 1000.0,
                "attributes": meta,
            }
        )
    return out


def parse_step_matrix_rows(df) -> list[dict[str, Any]]:
    """Normalize STEP_MATRIX_SQL query result."""
    if df is None or df.empty:
        return []
    out: list[dict[str, Any]] = []
    for _, row in df.iterrows():
        attrs = row.get("attributes", "{}")
        if isinstance(attrs, str):
            meta = json.loads(attrs) if attrs else {}
        else:
            meta = dict(attrs) if attrs else {}
        out.append(
            {
                "rank": int(meta.get("rank", -1)),
                "local_step": int(meta.get("local_step", -1)),
                "duration_ms": float(row.get("duration_us", 0)) / 1000.0,
                "attributes": meta,
            }
        )
    return out


def assert_rows_contain_fields(rows: Iterable[dict[str, Any]], **expected: Any) -> None:
    for key, value in expected.items():
        assert all(
            row.get(key) == value for row in rows
        ), f"expected all rows[{key}]=={value!r}, got {[row.get(key) for row in rows]}"

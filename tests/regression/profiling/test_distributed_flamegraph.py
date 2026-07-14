"""Distributed SPMD torch flamegraph — memtable seed + server-aligned SQL."""

from __future__ import annotations

import json

import pytest

# Mirrors probing/server/src/server/training.rs ``distributed_torch_trace_sql``.
LOCAL_DISTRIBUTED_TORCH_TRACE_SQL = """
SELECT
  CAST(rank AS BIGINT) AS rank,
  module,
  stage,
  CAST(duration AS DOUBLE) AS duration,
  CAST(allocated_delta AS DOUBLE) AS allocated_delta,
  CAST(max_allocated_delta AS DOUBLE) AS max_allocated_delta,
  CAST(allocated AS DOUBLE) AS allocated,
  CAST(max_allocated AS DOUBLE) AS max_allocated,
  CAST(local_step AS BIGINT) AS local_step
FROM python.torch_trace
WHERE local_step = {step}
  AND module <> 'None'
  AND (stage LIKE 'post %' OR stage LIKE 'pre %')
"""


@pytest.fixture(autouse=True)
def _reset_torch_trace_table():
    from probing.profiling.torch_probe import TorchTrace

    try:
        TorchTrace.drop()
    except Exception:
        pass
    TorchTrace.init_table()
    yield
    try:
        TorchTrace.drop()
    except Exception:
        pass
    TorchTrace.init_table()


@pytest.fixture
def sql_query():
    from probing import query

    def _run(expr: str):
        return query(expr.strip())

    return _run


def _save_trace(
    *,
    rank: int,
    local_step: int,
    module: str,
    stage: str,
    duration: float = 0.0,
    allocated_delta: float = 0.0,
    max_allocated_delta: float = 0.0,
) -> None:
    from probing.profiling.torch_probe import TorchTrace

    TorchTrace(
        local_step=local_step,
        rank=rank,
        world_size=2,
        module=module,
        stage=stage,
        duration=duration,
        allocated_delta=allocated_delta,
        max_allocated_delta=max_allocated_delta,
    ).save()


def _seed_spmd_step(*, local_step: int = 7) -> None:
    """Two ranks at one step: shared encoder path + rank-specific decoder paths."""
    rows = [
        (0, "encoder.block", "post forward", 0.01),
        (1, "encoder.block", "post forward", 0.01),
        (0, "decoder", "post forward", 0.02),
        (1, "decoder.head", "post forward", 0.008),
    ]
    for rank, module, stage, duration in rows:
        _save_trace(
            rank=rank,
            local_step=local_step,
            module=module,
            stage=stage,
            duration=duration,
        )


@pytest.mark.training_observability
class TestDistributedFlamegraphSql:
    def test_multi_rank_rows_at_same_step(self, sql_query):
        _seed_spmd_step(local_step=7)
        df = sql_query(LOCAL_DISTRIBUTED_TORCH_TRACE_SQL.format(step=7))
        assert not df.empty
        ranks = {int(r) for r in df["rank"].unique()}
        assert ranks == {0, 1}
        modules = set(df["module"].tolist())
        assert {"encoder.block", "decoder", "decoder.head"} <= modules

    def test_step_filter_excludes_other_steps(self, sql_query):
        _seed_spmd_step(local_step=7)
        _save_trace(
            rank=0, local_step=99, module="other", stage="post forward", duration=1.0
        )
        df = sql_query(LOCAL_DISTRIBUTED_TORCH_TRACE_SQL.format(step=7))
        assert int(df["local_step"].max()) == 7
        assert "other" not in set(df["module"].tolist())

    def test_post_stage_filter_excludes_unrelated_stages(self, sql_query):
        _save_trace(
            rank=0, local_step=3, module="m", stage="post forward", duration=0.01
        )
        _save_trace(rank=0, local_step=3, module="m", stage="running", duration=9.0)
        df = sql_query(LOCAL_DISTRIBUTED_TORCH_TRACE_SQL.format(step=3))
        assert len(df) == 1
        assert df.iloc[0]["stage"] == "post forward"


@pytest.mark.training_observability
class TestDistributedFlamegraphContract:
    def test_api_spec_lists_distributed_endpoint(self):
        from pathlib import Path

        spec_path = Path(__file__).resolve().parents[1] / "spec" / "api_spec.json"
        spec = json.loads(spec_path.read_text(encoding="utf-8"))
        paths = {(r["method"], r["path"]) for r in spec["server_public"]}
        assert ("GET", "/apis/training/distributed_flamegraph/json") in paths

    def test_web_client_declares_distributed_stack_path(self):
        from pathlib import Path

        spec_path = Path(__file__).resolve().parents[1] / "spec" / "api_spec.json"
        spec = json.loads(spec_path.read_text(encoding="utf-8"))
        stack_calls: list[str] = []
        for entry in spec["client_contracts"]["web"]:
            if entry["source"] != "web/src/api/stack.rs":
                continue
            stack_calls.extend(c["path"] for c in entry["calls"])
        assert "/apis/training/distributed_stack_flamegraph/json" in stack_calls

        # Legacy torch SPMD endpoint remains server-public; Web UI uses stack flamegraph.
        paths = {(r["method"], r["path"]) for r in spec["server_public"]}
        assert ("GET", "/apis/training/distributed_flamegraph/json") in paths

    @staticmethod
    def _normalize_profiling_view(view: str) -> str:
        view = view.strip().strip("/")
        if view in ("torch-dist", "distributed-torch", "torch-distributed"):
            return "torch-dist"
        return view or "pprof"

    def test_profiling_view_slug_aliases(self):
        assert self._normalize_profiling_view("distributed-torch") == "torch-dist"
        assert self._normalize_profiling_view("torch-distributed") == "torch-dist"

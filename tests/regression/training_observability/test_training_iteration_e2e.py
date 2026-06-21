"""End-to-end: one synthetic training iteration produces queryable observability data."""

import pytest

import probing
from probing.profiling.collective.record import CommCollective, record_comm_lite
from probing.tracing.phases import BACKWARD, FORWARD, OPTIMIZER

from .conftest import table_rows, train_step_samples_from_memtable


@pytest.mark.training_observability
class TestTrainingIterationPipeline:
    def test_single_iteration_step_and_comm(self, rank_env, parallel_env):
        rank_env(rank=1, world_size=8)
        parallel_env(tp_rank=0, pp_rank=1, dp_rank=1)
        probing.step(7)

        with probing.span("forward", phase=FORWARD):
            pass
        record_comm_lite(
            op="all_reduce",
            duration_ms=8.5,
            group_rank=1,
            group_size=8,
            nbytes=4096,
        )
        with probing.span("backward", phase=BACKWARD):
            pass
        with probing.span("optimizer", phase=OPTIMIZER):
            pass
        probing.record_span("train.step", duration_ns=int(120.0 * 1e6), source="test")

        step_rows = train_step_samples_from_memtable()
        assert len(step_rows) >= 1
        assert any(r["rank"] == 1 for r in step_rows)

        comm_rows = table_rows(CommCollective, 5)
        assert len(comm_rows) == 1
        assert comm_rows[0]["op"] == "all_reduce"
        assert "pp=1" in comm_rows[0]["role"]
        assert comm_rows[0]["bytes"] == 4096

    def test_event_on_training_span(self):
        with probing.span("forward", phase=FORWARD):
            probing.event("batch.stats", attributes=[{"loss": 1.25}])

    def test_torch_probe_reentrant_optimizer(self):
        from probing.profiling.torch_probe import TorchProbe, TorchProbeConfig

        tracer = TorchProbe(config=TorchProbeConfig(enabled=True))
        with probing.span("outer", phase=OPTIMIZER) as outer:
            tracer._begin_train_step_span()
            assert not outer.is_ended
            tracer._end_train_step_span()
            assert not outer.is_ended
            probing.event("still.open")
        assert outer.is_ended

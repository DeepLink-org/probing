"""End-to-end: one synthetic training iteration produces queryable observability data."""

import pytest

import probing
from probing.profiling.collective.record import record_comm_lite
from probing.tracing import TRAIN_STEP_KIND, sync_local_step

from .conftest import train_step_samples_from_memtable, table_rows
from probing.profiling.collective.record import CommCollective


@pytest.mark.training_observability
class TestTrainingIterationPipeline:
    def test_single_iteration_step_and_comm(self, rank_env, parallel_env):
        """Mimics: train.step → collective → memtable rows used by Training page."""
        rank_env(rank=1, world_size=8)
        parallel_env(tp_rank=0, pp_rank=1, dp_rank=1)
        sync_local_step(7)

        with probing.span("batch", kind=TRAIN_STEP_KIND):
            with probing.span("forward", kind="nn.forward"):
                pass
            record_comm_lite(
                op="all_reduce",
                duration_ms=8.5,
                group_rank=1,
                group_size=8,
                nbytes=4096,
            )
            with probing.span("backward", kind="nn.backward"):
                pass

        step_rows = train_step_samples_from_memtable()
        assert len(step_rows) >= 1
        assert any(r["rank"] == 1 and r["local_step"] == 7 for r in step_rows)

        comm_rows = table_rows(CommCollective, 5)
        assert len(comm_rows) == 1
        assert comm_rows[0]["op"] == "all_reduce"
        assert comm_rows[0]["pp_rank"] == 1
        assert comm_rows[0]["bytes"] == 4096

    def test_train_step_event_after_nested_spans(self):
        """Regression path from imagenet_with_span (SpanAlreadyClosed)."""
        with probing.span("batch", kind=TRAIN_STEP_KIND):
            with probing.span("forward", kind="nn.forward"):
                pass
            probing.event("batch.stats", attributes=[{"loss": 1.25}])

    def test_torch_probe_reentrant_train_step(self):
        from probing.profiling.torch_probe import TorchProbe, TorchProbeConfig

        tracer = TorchProbe(config=TorchProbeConfig(enabled=True))
        with probing.span("batch", kind=TRAIN_STEP_KIND) as outer:
            tracer._begin_train_step_span()
            assert not outer.is_ended
            tracer._end_train_step_span()
            assert not outer.is_ended
            probing.event("still.open")
        assert outer.is_ended

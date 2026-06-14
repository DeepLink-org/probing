"""Step straggler analytics: train.step spans joinable like the Training API."""

import pytest

from probing.tracing import TRAIN_STEP_KIND, record_closed_span, sync_local_step

from .conftest import train_step_samples_from_memtable


@pytest.mark.training_observability
class TestStepStragglerAnalytics:
    def test_empty_without_train_step_spans(self):
        assert train_step_samples_from_memtable() == []

    def test_train_step_duration_from_closed_spans(self, rank_env):
        sync_local_step(42)
        rank_env(rank=3, world_size=8)

        record_closed_span(
            "batch",
            kind=TRAIN_STEP_KIND,
            duration_ns=int(150.0 * 1e6),
            source="test",
        )

        rows = train_step_samples_from_memtable()
        assert len(rows) == 1
        assert rows[0]["rank"] == 3
        assert rows[0]["local_step"] == 42
        assert rows[0]["duration_ms"] == pytest.approx(150.0, rel=0.05)

    def test_multi_rank_straggler_simulation(self, rank_env):
        """Single process simulates cross-rank matrix by varying RANK env."""
        sync_local_step(100)
        durations = {0: 120.0, 1: 118.0, 2: 350.0, 3: 125.0}

        for rank, duration_ms in durations.items():
            rank_env(rank=rank, world_size=4)
            record_closed_span(
                "batch",
                kind=TRAIN_STEP_KIND,
                duration_ns=int(duration_ms * 1e6),
                source="test",
            )

        rows = train_step_samples_from_memtable()
        by_rank = {r["rank"]: r["duration_ms"] for r in rows}
        assert set(by_rank) == {0, 1, 2, 3}
        assert by_rank[2] > by_rank[0] * 2
        assert all(r["local_step"] == 100 for r in rows)

    def test_ignores_non_train_step_kinds(self):
        record_closed_span(
            "forward",
            kind="nn.forward",
            duration_ns=int(50.0 * 1e6),
        )

        assert train_step_samples_from_memtable() == []

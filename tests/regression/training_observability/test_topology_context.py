"""3D parallel topology flows into spans and collective rows."""

import pytest

import probing
from probing.parallel import parallel_fields, parallel_topology
from probing.profiling.collective.record import CommCollective, record_comm_lite
from probing.tracing import comm_kind

from .conftest import table_rows


@pytest.fixture(autouse=True)
def _clear_parallel_env(monkeypatch):
    for key in (
        "TENSOR_MODEL_PARALLEL_RANK",
        "PIPELINE_MODEL_PARALLEL_RANK",
        "DATA_PARALLEL_RANK",
        "TENSOR_MODEL_PARALLEL_SIZE",
        "PIPELINE_MODEL_PARALLEL_SIZE",
        "DATA_PARALLEL_SIZE",
        "TP_RANK",
        "PP_RANK",
        "DP_RANK",
    ):
        monkeypatch.delenv(key, raising=False)
    yield


@pytest.mark.training_observability
class TestParallelTopology:
    def test_megatron_env_parsed(self, monkeypatch):
        monkeypatch.setenv("TENSOR_MODEL_PARALLEL_RANK", "2")
        monkeypatch.setenv("PIPELINE_MODEL_PARALLEL_RANK", "1")
        monkeypatch.setenv("DATA_PARALLEL_RANK", "3")
        monkeypatch.setenv("TENSOR_MODEL_PARALLEL_SIZE", "8")
        monkeypatch.setenv("PIPELINE_MODEL_PARALLEL_SIZE", "4")
        monkeypatch.setenv("DATA_PARALLEL_SIZE", "16")

        topo = parallel_topology()
        assert topo.tp_rank == 2
        assert topo.pp_rank == 1
        assert topo.dp_rank == 3
        assert topo.tp_size == 8
        assert topo.pp_size == 4
        assert topo.dp_size == 16

    def test_unset_fields_omitted(self):
        assert parallel_fields() == {}

    def test_span_includes_topology(self, monkeypatch):
        monkeypatch.setenv("TP_RANK", "1")
        monkeypatch.setenv("PP_RANK", "0")
        monkeypatch.setenv("DP_RANK", "5")

        with probing.span("op") as s:
            attrs = dict(s.get_attributes())
            assert attrs["tp_rank"] == 1
            assert attrs["pp_rank"] == 0
            assert attrs["dp_rank"] == 5

    def test_comm_kind_labels(self):
        assert comm_kind("all_reduce") == "comm.all_reduce"
        assert comm_kind("comm.broadcast") == "comm.broadcast"


@pytest.mark.training_observability
class TestTopologyInCollectiveRows:
    def test_comm_lite_row_carries_role(self, parallel_env, rank_env):
        rank_env(rank=2, world_size=8)
        parallel_env(tp_rank=1, pp_rank=0, dp_rank=2)

        record_comm_lite(
            op="all_reduce",
            duration_ms=4.0,
            group_rank=2,
            group_size=8,
            nbytes=2048,
        )

        row = table_rows(CommCollective, 1)[0]
        assert row["role"] == "dp=2,pp=0,tp=1"
        assert row["rank"] == 2

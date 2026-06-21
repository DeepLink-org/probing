import os

import pytest

from probing.parallel import parallel_fields, parallel_topology


@pytest.fixture(autouse=True)
def _clear_parallel_env(monkeypatch):
    keys = [
        "TENSOR_MODEL_PARALLEL_RANK",
        "PIPELINE_MODEL_PARALLEL_RANK",
        "DATA_PARALLEL_RANK",
        "TENSOR_MODEL_PARALLEL_SIZE",
        "PIPELINE_MODEL_PARALLEL_SIZE",
        "DATA_PARALLEL_SIZE",
        "TP_RANK",
        "PP_RANK",
        "DP_RANK",
    ]
    for key in keys:
        monkeypatch.delenv(key, raising=False)
    yield


def test_parallel_topology_from_megatron_env(monkeypatch):
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


def test_parallel_fields_omits_unset():
    assert parallel_fields() == {}


def test_span_includes_parallel_fields(monkeypatch):
    monkeypatch.setenv("TP_RANK", "1")
    monkeypatch.setenv("PP_RANK", "0")
    monkeypatch.setenv("DP_RANK", "5")

    import probing

    with probing.span("op") as s:
        attrs = dict(s.get_attributes())
        assert attrs["tp_rank"] == 1
        assert attrs["pp_rank"] == 0
        assert attrs["dp_rank"] == 5


def test_comm_label():
    from probing.profiling.collective.record import _comm_label

    assert _comm_label("all_reduce") == "comm.all_reduce"
    assert _comm_label("comm.broadcast") == "comm.broadcast"

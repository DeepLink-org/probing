"""Pure topology checks for the executable 64-rank Megatron example."""

from __future__ import annotations

import importlib.util
import sys
from pathlib import Path


def _example_module():
    path = (
        Path(__file__).resolve().parents[4]
        / "examples"
        / "megatron"
        / "megatron_64_rank_mock.py"
    )
    spec = importlib.util.spec_from_file_location("megatron_64_rank_mock", path)
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    sys.modules[spec.name] = module
    try:
        spec.loader.exec_module(module)
    finally:
        sys.modules.pop(spec.name, None)
    return module


def test_64_rank_megatron_example_has_expected_physical_and_parallel_layout():
    example = _example_module()
    topology = example.build_topology()

    example.validate_topology(topology)

    assert topology[0].role == "dp=0,pp=0,sp=0,tp=0"
    assert topology[63].role == "dp=7,pp=3,sp=1,tp=1"
    assert topology[63].host == "megatron-node-07"
    assert topology[63].local_rank == 7


def test_sequence_parallel_reuses_tensor_parallel_group():
    example = _example_module()
    groups = example.groups_for_rank(example.build_topology(), 0)

    assert groups == {
        "tp": [0, 1],
        "pp": [0, 2, 4, 6],
        "dp": [0, 8, 16, 24, 32, 40, 48, 56],
        "sp": [0, 1],
    }


def test_fixture_wait_counter_recorder_reports_elapsed_calls():
    example = _example_module()
    recorder = example.WaitCounterRecorder()

    with recorder.wait("pytorch.wait_counter.fixture.ProcessGroupPP__recv"):
        pass

    counter = recorder.snapshot()["pytorch.wait_counter.fixture.ProcessGroupPP__recv"]
    assert counter["active_count"] == 0
    assert counter["total_calls"] == 1
    assert counter["total_time_us"] >= 1
    assert counter["max_time_us"] >= 1


def test_64_rank_launcher_uses_64_worker_torchrun():
    root = Path(__file__).resolve().parents[4]
    launcher = (root / "examples" / "megatron" / "run_64_rank_mock.sh").read_text()

    assert '"$PYTHON" -m torch.distributed.run' in launcher
    assert "--nproc-per-node=64" in launcher
    assert "--no-python" in launcher
    assert "examples/megatron/run_64_rank_worker.sh" in launcher
    assert "export PROBING_TORCHRUN_CLUSTER=1" in launcher
    assert '--master-addr="$MASTER_ADDR"' in launcher
    assert '--master-port="$MASTER_PORT"' in launcher


def test_worker_launcher_configures_logical_node_before_python_starts():
    root = Path(__file__).resolve().parents[4]
    worker = (root / "examples" / "megatron" / "run_64_rank_worker.sh").read_text()

    assert "logical_local_rank=$((RANK % 8))" in worker
    assert "logical_node_rank=$((RANK / 8))" in worker
    assert 'export LOCAL_RANK="$logical_local_rank"' in worker
    assert 'export GROUP_RANK="$logical_node_rank"' in worker
    assert 'exec "$PROBING_MOCK_PYTHON"' in worker


def test_worker_runtime_maps_real_ranks_to_eight_logical_hosts(monkeypatch):
    example = _example_module()
    topology = example.build_topology()
    monkeypatch.setenv("RANK", "17")

    worker = example._configure_worker(topology)

    assert worker.rank == 17
    assert worker.host == "megatron-node-02"
    assert example.os.environ["LOCAL_RANK"] == "1"
    assert example.os.environ["GROUP_RANK"] == "2"
    assert example.os.environ["PROBING_NODE_HOST"] == "megatron-node-02"
    assert example.os.environ["PROBING_NODE_ROLE"] == "dp=2,pp=0,sp=1,tp=1"

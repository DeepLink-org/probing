"""Unit tests for probing.fakes (no real Megatron / CUDA)."""

from __future__ import annotations

import sys

import pytest

torch = pytest.importorskip("torch")


@pytest.fixture(autouse=True)
def _clean_fakes():
    from probing import fakes
    from probing.ext import megatron as megatron_ext
    from probing.fakes.journal import FakeEvent, begin_run

    megatron_ext._PARALLEL_STATE_INIT = False
    megatron_ext._TRAINING_INIT = False
    megatron_ext._LAST_ROLE = None
    megatron_ext._LAST_ITERATION = None
    megatron_ext._RECORDED_RANK_LAYOUTS.clear()
    megatron_ext._CAPTURED_MODEL_IDS.clear()
    if fakes.is_installed():
        fakes.uninstall()
    try:
        FakeEvent.drop()
    except Exception:
        pass
    try:
        FakeEvent.init_table()
    except Exception:
        pass
    begin_run("test-clean")
    yield
    if fakes.is_installed():
        fakes.uninstall()
    megatron_ext._PARALLEL_STATE_INIT = False
    megatron_ext._TRAINING_INIT = False
    megatron_ext._RECORDED_RANK_LAYOUTS.clear()
    megatron_ext._CAPTURED_MODEL_IDS.clear()


def test_device_remap_cuda_to_meta():
    from probing.fakes import install, uninstall, target_device

    install(specs=("megatron",), device="meta")
    assert target_device() == "meta"
    assert torch.cuda.is_available() is True

    t = torch.zeros(4, device="cuda")
    assert t.device.type == "meta"

    m = torch.nn.Linear(3, 2).cuda()
    assert next(m.parameters()).device.type == "meta"

    uninstall()
    # After uninstall, cuda availability is whatever the host reports.
    assert callable(torch.cuda.is_available)


def test_megatron_import_via_finder(monkeypatch):
    monkeypatch.delenv("PROBING_FAKES", raising=False)
    from probing.fakes import install

    # force=True: this machine may have a broken megatron-core (no triton).
    install(specs=("megatron",), remap_device=False, force=True)

    from megatron.core import parallel_state
    from megatron.training.training import train_step

    assert getattr(parallel_state, "__probing_fake__", False) is True
    assert parallel_state.model_parallel_is_initialized() is True
    assert train_step() == {"loss": 1.0}


def test_skip_real_package(monkeypatch):
    """Finder must not shadow an already-importable real top-level package."""
    import types

    from probing.fakes.finder import FakeFinder, install_finder, uninstall_finder
    from probing.fakes import registry

    # Use a throwaway prefix that we make "real" via a custom finder underneath.
    real = types.ModuleType("probing_fakes_real_sentinel")
    real.__probing_fake__ = False  # type: ignore[attr-defined]
    sys.modules["probing_fakes_real_sentinel"] = real

    def factory(fullname: str) -> types.ModuleType:
        mod = types.ModuleType(fullname)
        mod.__probing_fake__ = True  # type: ignore[attr-defined]
        return mod

    registry.register(
        registry.FakeSpec(
            name="probing_fakes_real_sentinel",
            prefixes=("probing_fakes_real_sentinel",),
            factory=factory,
        )
    )
    registry.enable({"probing_fakes_real_sentinel"})
    install_finder()
    try:
        import probing_fakes_real_sentinel as mod

        assert getattr(mod, "__probing_fake__", False) is False
    finally:
        uninstall_finder()
        registry.disable_all()
        sys.modules.pop("probing_fakes_real_sentinel", None)
        registry._SPECS.pop("probing_fakes_real_sentinel", None)


def test_scripted_loop_syncs_role_and_step(monkeypatch):
    import probing
    from probing.fakes import run_scripted_loop

    monkeypatch.setenv("PROBING_MEGATRON", "on")
    monkeypatch.setenv("PROBING_MEGATRON_STEP_SYNC", "on")
    probing.step(0, micro_batches=1)

    result = run_scripted_loop(
        steps=3, tp=2, pp=1, dp=3, micro_batches=2, device="meta"
    )
    assert result.device == "meta"
    assert result.role == "dp=3,pp=1,tp=2"
    assert result.last_iteration == 2
    # sync uses micro_step = iteration * micro_batches → local_step == iteration
    assert probing.step.local_step == 2
    assert int(probing.step.snapshot().micro_batches) == 2


@pytest.mark.parametrize(
    ("tp", "pp", "dp", "expected_role"),
    [
        (0, 0, 0, "dp=0,pp=0,tp=0"),
        (1, 0, 0, "dp=0,pp=0,tp=1"),
        (0, 1, 0, "dp=0,pp=1,tp=0"),
        (1, 1, 0, "dp=0,pp=1,tp=1"),
    ],
)
def test_cpu_mock_emits_training_placement_coordinates(
    monkeypatch, tp, pp, dp, expected_role
):
    from probing.fakes import run_scripted_loop

    monkeypatch.setenv("PROBING_MEGATRON", "on")
    monkeypatch.setenv("PROBING_MEGATRON_STEP_SYNC", "on")

    result = run_scripted_loop(steps=1, tp=tp, pp=pp, dp=dp, device="cpu")

    assert result.device == "cpu"
    assert result.role == expected_role


def test_cpu_mock_emits_64_rank_tp2_pp4_dp8_topology(monkeypatch):
    from probing.ext import megatron as megatron_ext
    from probing.fakes import run_scripted_loop

    monkeypatch.setenv("PROBING_MEGATRON", "on")
    monkeypatch.setenv("PROBING_MEGATRON_STEP_SYNC", "on")
    roles = []

    for rank in range(64):
        megatron_ext._PARALLEL_STATE_INIT = False
        megatron_ext._TRAINING_INIT = False
        megatron_ext._LAST_ROLE = None
        tp = rank % 2
        pp = (rank // 2) % 4
        dp = rank // 8
        result = run_scripted_loop(steps=1, tp=tp, pp=pp, dp=dp, device="cpu")
        roles.append(result.role)

    assert len(set(roles)) == 64
    assert roles[0] == "dp=0,pp=0,tp=0"
    assert roles[63] == "dp=7,pp=3,tp=1"


def test_pretrain_gpt_entry(monkeypatch):
    import probing
    from pathlib import Path

    monkeypatch.setenv("PROBING_MEGATRON", "on")
    monkeypatch.setenv("PROBING_MEGATRON_STEP_SYNC", "on")
    probing.step(0, micro_batches=1)

    # Import the example module as a library and call main().
    import runpy

    script = (
        Path(__file__).resolve().parents[4]
        / "examples"
        / "megatron"
        / "pretrain_gpt.py"
    )
    assert script.is_file()
    ns = runpy.run_path(str(script), run_name="not_main")
    rc = ns["main"](
        [
            "--train-iters",
            "3",
            "--hidden-size",
            "32",
            "--vocab-size",
            "128",
        ]
    )
    assert rc == 0
    assert probing.step.local_step == 2


def test_maybe_install_from_env(monkeypatch):
    from probing import fakes

    monkeypatch.setenv("PROBING_FAKES", "megatron")
    monkeypatch.setenv("PROBING_FAKE_DEVICE", "cpu")
    assert fakes.maybe_install_from_env() is True
    assert fakes.is_installed()
    assert fakes.target_device() == "cpu"


def test_fake_event_correlates_with_probing_step(monkeypatch):
    import dataclasses

    import probing
    from probing.fakes import (
        FakeEvent,
        current_run_id,
        install,
        run_scripted_loop,
        verify_against_probing,
    )

    monkeypatch.setenv("PROBING_MEGATRON", "on")
    monkeypatch.setenv("PROBING_MEGATRON_STEP_SYNC", "on")
    probing.step(0, micro_batches=1)
    FakeEvent.init_table()

    install(force=True, device="meta")
    rid = current_run_id()
    run_scripted_loop(steps=3, tp=1, pp=0, dp=0, micro_batches=2, device="meta")

    raw = FakeEvent.take(200)
    fields = [f.name for f in dataclasses.fields(FakeEvent)]
    rows = [dict(zip(fields, data)) for _ts, data in raw]
    train_rows = [
        r for r in rows if r["kind"] == "train_step" and r.get("run_id") == rid
    ]
    assert len(train_rows) == 3

    report = verify_against_probing(
        require_train_steps=3, check_collectives=False, run_id=rid
    )
    assert report.ok, report.issues


def test_dist_hooks_dual_write_collective(monkeypatch):
    import dataclasses

    from probing.fakes import FakeEvent, current_run_id, install, verify_against_probing
    from probing.fakes.torch_hooks import install_torch_dist_hooks
    from probing.profiling.collective.record import CommCollective

    install(force=True, device="meta", specs=("megatron",))
    rid = current_run_id()
    assert install_torch_dist_hooks() is True

    FakeEvent.init_table()
    CommCollective.init_table()

    import torch
    import torch.distributed as dist

    t = torch.zeros(8, device="cuda")
    dist.all_reduce(t)  # not initialized → simulated

    fake_rows = [
        dict(zip([f.name for f in dataclasses.fields(FakeEvent)], data))
        for _ts, data in FakeEvent.take(50)
    ]
    assert any(
        r["kind"] == "collective"
        and r["name"] == "all_reduce"
        and r.get("run_id") == rid
        for r in fake_rows
    )

    comm_rows = [
        dict(zip([f.name for f in dataclasses.fields(CommCollective)], data))
        for _ts, data in CommCollective.take(50)
    ]
    assert any(r["op"] == "all_reduce" for r in comm_rows)

    report = verify_against_probing(
        check_step_alignment=False, check_collectives=True, run_id=rid
    )
    assert report.ok, report.issues


def test_device_remap_int_index_is_cuda_ordinal():
    """``torch.cuda.current_device()`` returns an int — factories must remap it."""
    from probing.fakes import install, uninstall

    install(specs=("triton",), device="cpu")
    try:
        t = torch.zeros(2, device=torch.cuda.current_device())
        assert t.device.type == "cpu"
        assert torch.cuda.Stream().device.type == "cpu"
    finally:
        uninstall()


def test_helpers_cpp_fallback_build_sample_idx():
    from probing.fakes.helpers_cpp import build_sample_idx_int32
    import numpy as np

    sizes = np.array([64, 64], dtype=np.int32)
    docs = np.array([0, 1, 0], dtype=np.int32)
    idx = build_sample_idx_int32(sizes, docs, 32, 1, 128, True, 1)
    assert idx.ndim == 2 and idx.shape[1] == 2
    assert idx.shape[0] >= 2


def test_merge_argv_overrides_valued_flags():
    from probing.fakes.megatron_lm import _merge_argv

    merged = _merge_argv(
        ["--train-iters", "2", "--mock-data"],
        ["--train-iters", "5", "--transformer-impl", "local"],
    )
    assert merged[merged.index("--train-iters") + 1] == "5"
    assert "--transformer-impl" in merged
    assert merged[merged.index("--transformer-impl") + 1] == "local"
    assert "--mock-data" in merged

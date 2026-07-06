"""Megatron autostart integration tests (mocked Megatron modules)."""

from __future__ import annotations

import sys
import types
from unittest import mock

import pytest

import probing
from probing.ext import megatron as megatron_ext


@pytest.fixture(autouse=True)
def _reset_megatron_state():
    megatron_ext._PARALLEL_STATE_INIT = False
    megatron_ext._TRAINING_INIT = False
    megatron_ext._LAST_ROLE = None
    megatron_ext._LAST_ITERATION = None
    probing.clear_role()
    for key in list(sys.modules):
        if key == "megatron" or key.startswith("megatron."):
            del sys.modules[key]
    yield
    probing.clear_role()


@pytest.fixture
def megatron_env(monkeypatch):
    monkeypatch.setenv("PROBING_MEGATRON", "on")
    monkeypatch.setenv("PROBING_MEGATRON_STEP_SYNC", "on")


def _make_parallel_state(
    *,
    initialized: bool = True,
    tp: int = 2,
    pp: int = 1,
    dp: int = 3,
):
    ps = types.ModuleType("megatron.core.parallel_state")

    def model_parallel_is_initialized():
        return initialized

    def get_tensor_model_parallel_rank():
        return tp

    def get_pipeline_model_parallel_rank():
        return pp

    def get_data_parallel_rank():
        return dp

    def initialize_model_parallel(*args, **kwargs):
        return None

    ps.model_parallel_is_initialized = model_parallel_is_initialized
    ps.get_tensor_model_parallel_rank = get_tensor_model_parallel_rank
    ps.get_pipeline_model_parallel_rank = get_pipeline_model_parallel_rank
    ps.get_data_parallel_rank = get_data_parallel_rank
    ps.initialize_model_parallel = initialize_model_parallel
    return ps


def test_megatron_job_detected_from_env(monkeypatch):
    monkeypatch.setenv("TENSOR_MODEL_PARALLEL_RANK", "0")
    assert megatron_ext.megatron_job_detected()


def test_sync_role_from_parallel_state(megatron_env):
    ps = _make_parallel_state(tp=2, pp=1, dp=3)
    role = megatron_ext.sync_role_from_parallel_state(ps)
    assert role == "dp=3,pp=1,tp=2"
    assert probing.current_role() == "dp=3,pp=1,tp=2"


def test_init_parallel_state_wraps_initialize(megatron_env, monkeypatch):
    ps = _make_parallel_state(tp=0, pp=0, dp=1)
    core = types.ModuleType("megatron.core")
    core.parallel_state = ps
    megatron = types.ModuleType("megatron")
    megatron.core = core
    monkeypatch.setitem(sys.modules, "megatron", megatron)
    monkeypatch.setitem(sys.modules, "megatron.core", core)
    monkeypatch.setitem(sys.modules, "megatron.core.parallel_state", ps)

    with mock.patch.dict(
        "sys.modules",
        {
            "megatron.core.parallel_state": ps,
        },
    ):
        megatron_ext.init_parallel_state()
        ps.initialize_model_parallel()
        assert probing.current_role() == "dp=1,pp=0,tp=0"


def test_train_step_wrap_syncs_iteration(megatron_env, monkeypatch):
    ps = _make_parallel_state()
    training_mod = types.ModuleType("megatron.training.training")
    calls: list[int] = []

    def train_step(*args, **kwargs):
        calls.append(1)
        return {"loss": 1.0}

    training_mod.train_step = train_step

    global_vars = types.ModuleType("megatron.training.global_vars")
    args_obj = types.SimpleNamespace(iteration=42)

    def get_args():
        return args_obj

    global_vars.get_args = get_args

    num_calc = types.ModuleType("megatron.core.num_microbatches_calculator")

    def get_num_microbatches():
        return 4

    num_calc.get_num_microbatches = get_num_microbatches

    core = types.ModuleType("megatron.core")
    core.parallel_state = ps
    core.num_microbatches_calculator = num_calc
    training_pkg = types.ModuleType("megatron.training")
    training_pkg.training = training_mod
    training_pkg.global_vars = global_vars
    megatron = types.ModuleType("megatron")
    megatron.core = core
    megatron.training = training_pkg

    modules = {
        "megatron": megatron,
        "megatron.core": core,
        "megatron.core.parallel_state": ps,
        "megatron.core.num_microbatches_calculator": num_calc,
        "megatron.training": training_pkg,
        "megatron.training.training": training_mod,
        "megatron.training.global_vars": global_vars,
    }
    with mock.patch.dict(sys.modules, modules):
        megatron_ext.init_training()
        training_mod.train_step()
        assert calls == [1]
        assert probing.step.local_step == 42
        assert int(probing.step.snapshot().micro_batches) == 4


def test_megatron_disabled_by_env(monkeypatch):
    monkeypatch.setenv("PROBING_MEGATRON", "off")
    monkeypatch.setenv("TENSOR_MODEL_PARALLEL_RANK", "0")
    assert not megatron_ext.megatron_autostart_enabled()

"""Megatron contract tests (mocked modules, fast CI).

Validates import-hook wiring and role/step sync against the in-process probing
engine path without requiring ``megatron-core``.
"""

from __future__ import annotations

import sys
import types
from unittest import mock

import pytest

import probing
from probing.ext import megatron as megatron_ext
from probing.hooks import import_hook

from ._megatron_contract import (
    MEGATRON_HOOK_MODULES,
    install_megatron_stack,
    make_parallel_state,
    megatron_modules,
)


def test_import_hook_registers_megatron_callbacks():
    for name in MEGATRON_HOOK_MODULES:
        assert name in import_hook.register, f"missing import hook for {name}"
        assert callable(import_hook.register[name])


def test_megatron_usability_end_to_end(megatron_env, monkeypatch):
    monkeypatch.setenv("TENSOR_MODEL_PARALLEL_RANK", "2")
    monkeypatch.setenv("PIPELINE_MODEL_PARALLEL_RANK", "1")
    monkeypatch.setenv("DATA_PARALLEL_RANK", "3")

    ps = make_parallel_state(tp=2, pp=1, dp=3)
    modules = install_megatron_stack(ps=ps, iteration=10, micro_batches=4)
    training_mod = modules["megatron.training.training"]
    args_obj = modules["megatron.training.global_vars"]._probing_args  # type: ignore[attr-defined]

    with megatron_modules(modules):
        assert megatron_ext.megatron_autostart_enabled()

        megatron_ext.init_parallel_state()
        assert probing.current_role() == "dp=3,pp=1,tp=2"

        megatron_ext.init_training()
        assert getattr(training_mod.train_step, "_probing_wrapped", False)

        training_mod.train_step()
        assert training_mod._probing_train_calls == [1]  # type: ignore[attr-defined]
        assert probing.step.local_step == 10
        assert int(probing.step.snapshot().micro_batches) == 4

        args_obj.iteration = 11
        training_mod.train_step()
        assert probing.step.local_step == 11

        from probing.tracing.coordinates import row_fields

        fields = row_fields()
        assert fields["local_step"] == 11
        assert fields["micro_batches"] == 4


def test_maybe_autostart_when_megatron_loaded_first(megatron_env):
    ps = make_parallel_state(tp=1, pp=0, dp=7)
    modules = install_megatron_stack(ps=ps, iteration=5, micro_batches=2)

    with megatron_modules(modules):
        sys.modules["megatron.core.parallel_state"] = ps
        sys.modules["megatron.training.training"] = modules[
            "megatron.training.training"
        ]

        megatron_ext.maybe_autostart()

        assert probing.current_role() == "dp=7,pp=0,tp=1"
        training_mod = modules["megatron.training.training"]
        assert getattr(training_mod.train_step, "_probing_wrapped", False)


def test_megatron_job_detected_from_env(monkeypatch):
    monkeypatch.setenv("TENSOR_MODEL_PARALLEL_RANK", "0")
    assert megatron_ext.megatron_job_detected()


def test_sync_role_from_parallel_state(megatron_env):
    ps = make_parallel_state(tp=2, pp=1, dp=3)
    role = megatron_ext.sync_role_from_parallel_state(ps)
    assert role == "dp=3,pp=1,tp=2"
    assert probing.current_role() == "dp=3,pp=1,tp=2"


def test_init_parallel_state_wraps_initialize(megatron_env, monkeypatch):
    ps = make_parallel_state(tp=0, pp=0, dp=1)
    core = types.ModuleType("megatron.core")
    core.parallel_state = ps
    megatron = types.ModuleType("megatron")
    megatron.core = core
    monkeypatch.setitem(sys.modules, "megatron", megatron)
    monkeypatch.setitem(sys.modules, "megatron.core", core)
    monkeypatch.setitem(sys.modules, "megatron.core.parallel_state", ps)

    with mock.patch.dict("sys.modules", {"megatron.core.parallel_state": ps}):
        megatron_ext.init_parallel_state()
        ps.initialize_model_parallel()
        assert probing.current_role() == "dp=1,pp=0,tp=0"


def test_train_step_wrap_syncs_iteration(megatron_env):
    ps = make_parallel_state()
    training_mod = types.ModuleType("megatron.training.training")
    calls: list[int] = []

    def train_step(*args, **kwargs):
        calls.append(1)
        return {"loss": 1.0}

    training_mod.train_step = train_step

    global_vars = types.ModuleType("megatron.training.global_vars")
    args_obj = types.SimpleNamespace(iteration=42)
    global_vars.get_args = lambda: args_obj

    num_calc = types.ModuleType("megatron.core.num_microbatches_calculator")
    num_calc.get_num_microbatches = lambda: 4

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


def test_sync_step_from_iteration(megatron_env):
    megatron_ext.sync_step_from_iteration(7, micro_batches=4, force=True)
    assert probing.step.local_step == 7
    assert probing.step.micro_step == 28


def test_megatron_disabled_by_env(monkeypatch):
    monkeypatch.setenv("PROBING_MEGATRON", "off")
    monkeypatch.setenv("TENSOR_MODEL_PARALLEL_RANK", "0")
    assert not megatron_ext.megatron_autostart_enabled()

"""vLLM contract tests (mocked modules, fast CI).

Validates import-hook wiring and role/step sync without requiring a real vLLM
install or GPU/Metal backend.
"""

from __future__ import annotations

import sys
import types

import pytest

import probing
from probing.ext import vllm as vllm_ext
from probing.hooks import import_hook

from ._vllm_contract import (
    VLLM_HOOK_MODULES,
    install_vllm_stack,
    make_llm_engine_class,
    vllm_modules,
)


def test_import_hook_registers_vllm_callbacks():
    for name in VLLM_HOOK_MODULES:
        assert name in import_hook.register, f"missing import hook for {name}"
        assert callable(import_hook.register[name])


def test_vllm_usability_end_to_end(vllm_env, monkeypatch):
    monkeypatch.setenv("RANK", "2")
    monkeypatch.setenv("TENSOR_MODEL_PARALLEL_RANK", "1")
    monkeypatch.setenv("VLLM_MLX_DEVICE", "gpu")

    modules = install_vllm_stack(engine_module="vllm.v1.engine.llm_engine")
    engine_mod = modules["vllm.v1.engine.llm_engine"]
    engine_cls = engine_mod.LLMEngine

    with vllm_modules(modules):
        assert vllm_ext.vllm_autostart_enabled()
        assert vllm_ext.metal_backend_detected()

        vllm_ext.init_metal_platform()
        assert probing.current_role() == "backend=metal,mlx=gpu,rank=2,tp=1"

        vllm_ext.init_v1_engine()
        assert getattr(engine_cls.step, "_probing_wrapped", False)

        engine = engine_cls()
        engine.step()
        assert engine.step_counter == 1
        assert probing.step.local_step == 1

        engine.step()
        assert probing.step.local_step == 2

        from probing.tracing.coordinates import row_fields

        fields = row_fields()
        assert fields["local_step"] == 2


def test_maybe_autostart_when_vllm_loaded_first(vllm_env, monkeypatch):
    monkeypatch.setenv("LOCAL_RANK", "0")
    monkeypatch.setenv("VLLM_METAL_USE_MLX", "1")

    modules = install_vllm_stack(engine_module="vllm.engine.llm_engine")
    engine_mod = modules["vllm.engine.llm_engine"]
    engine_cls = engine_mod.LLMEngine

    with vllm_modules(modules):
        sys.modules["vllm_metal"] = modules["vllm_metal"]
        sys.modules["vllm.engine.llm_engine"] = engine_mod

        vllm_ext.maybe_autostart()

        assert probing.current_role() == "backend=metal,rank=0"
        assert getattr(engine_cls.step, "_probing_wrapped", False)


def test_vllm_job_detected_from_env(monkeypatch):
    monkeypatch.setenv("VLLM_USE_V1", "1")
    assert vllm_ext.vllm_job_detected()


def test_metal_backend_detected_from_env(monkeypatch):
    monkeypatch.setenv("VLLM_MLX_DEVICE", "gpu")
    assert vllm_ext.metal_backend_detected()


def test_sync_role_from_env(vllm_env, monkeypatch):
    monkeypatch.setenv("RANK", "3")
    monkeypatch.setenv("DATA_PARALLEL_RANK", "1")
    monkeypatch.setenv("VLLM_MLX_DEVICE", "cpu")

    role = vllm_ext.sync_role_from_env()
    assert role == "backend=metal,dp=1,mlx=cpu,rank=3"
    assert probing.current_role() == "backend=metal,dp=1,mlx=cpu,rank=3"


def test_engine_step_wrap_syncs_counter(vllm_env):
    engine_mod = types.ModuleType("vllm.v1.engine.llm_engine")
    engine_cls = make_llm_engine_class(initial_step=10)
    engine_mod.LLMEngine = engine_cls

    v1_engine_pkg = types.ModuleType("vllm.v1.engine")
    v1_engine_pkg.llm_engine = engine_mod
    v1_pkg = types.ModuleType("vllm.v1")
    v1_pkg.engine = v1_engine_pkg
    vllm = types.ModuleType("vllm")
    vllm.v1 = v1_pkg

    modules = {
        "vllm": vllm,
        "vllm.v1": v1_pkg,
        "vllm.v1.engine": v1_engine_pkg,
        "vllm.v1.engine.llm_engine": engine_mod,
    }
    with vllm_modules(modules):
        vllm_ext.init_v1_engine()
        engine = engine_cls()
        engine.step()
        assert engine.step_counter == 11
        assert probing.step.local_step == 11


def test_sync_step_from_llm(vllm_env):
    class _Engine:
        step_counter = 9

    class _LLM:
        llm_engine = _Engine()

    vllm_ext.sync_step_from_llm(_LLM(), force=True)
    assert probing.step.local_step == 9


def test_vllm_disabled_by_env(monkeypatch):
    monkeypatch.setenv("PROBING_VLLM", "off")
    monkeypatch.setenv("VLLM_USE_V1", "1")
    assert not vllm_ext.vllm_autostart_enabled()

"""Shared vLLM mock helpers for contract tests (not collected by pytest)."""

from __future__ import annotations

import sys
import types
from contextlib import contextmanager
from unittest import mock

VLLM_HOOK_MODULES = (
    "vllm_metal",
    "vllm.engine.llm_engine",
    "vllm.v1.engine.llm_engine",
)


def make_llm_engine_class(*, initial_step: int = 0) -> type:
    class LLMEngine:
        def __init__(self) -> None:
            self.step_counter = initial_step

        def step(self):
            self.step_counter += 1
            return []

    return LLMEngine


def install_vllm_stack(
    *,
    engine_module: str = "vllm.v1.engine.llm_engine",
    initial_step: int = 0,
    with_metal: bool = True,
) -> dict[str, types.ModuleType]:
    engine_mod = types.ModuleType(engine_module)
    engine_cls = make_llm_engine_class(initial_step=initial_step)
    engine_mod.LLMEngine = engine_cls

    v1_engine_pkg = types.ModuleType("vllm.v1.engine")
    v1_engine_pkg.llm_engine = engine_mod
    v1_pkg = types.ModuleType("vllm.v1")
    v1_pkg.engine = v1_engine_pkg

    classic_engine_mod = types.ModuleType("vllm.engine.llm_engine")
    classic_engine_mod.LLMEngine = make_llm_engine_class(initial_step=initial_step)

    engine_pkg = types.ModuleType("vllm.engine")
    engine_pkg.llm_engine = classic_engine_mod

    vllm = types.ModuleType("vllm")
    vllm.v1 = v1_pkg
    vllm.engine = engine_pkg

    modules: dict[str, types.ModuleType] = {
        "vllm": vllm,
        "vllm.v1": v1_pkg,
        "vllm.v1.engine": v1_engine_pkg,
        "vllm.engine": engine_pkg,
        "vllm.engine.llm_engine": classic_engine_mod,
        engine_module: engine_mod,
    }

    if with_metal:
        metal_mod = types.ModuleType("vllm_metal")
        metal_mod.register = lambda: "vllm_metal.platform.MetalPlatform"
        modules["vllm_metal"] = metal_mod

    return modules


@contextmanager
def vllm_modules(modules: dict[str, types.ModuleType]):
    with mock.patch.dict(sys.modules, modules):
        yield modules

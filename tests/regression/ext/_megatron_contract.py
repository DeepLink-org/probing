"""Shared Megatron mock helpers for contract tests (not collected by pytest)."""

from __future__ import annotations

import sys
import types
from contextlib import contextmanager
from types import SimpleNamespace
from unittest import mock

MEGATRON_HOOK_MODULES = (
    "megatron.core.parallel_state",
    "megatron.training.training",
)


def make_parallel_state(
    *,
    initialized: bool = True,
    tp: int = 2,
    pp: int = 1,
    dp: int = 3,
) -> types.ModuleType:
    ps = types.ModuleType("megatron.core.parallel_state")

    def model_parallel_is_initialized() -> bool:
        return initialized

    ps.model_parallel_is_initialized = model_parallel_is_initialized
    ps.get_tensor_model_parallel_rank = lambda: tp
    ps.get_pipeline_model_parallel_rank = lambda: pp
    ps.get_data_parallel_rank = lambda: dp
    ps.initialize_model_parallel = lambda *args, **kwargs: None
    return ps


def install_megatron_stack(
    *,
    ps: types.ModuleType,
    iteration: int = 42,
    micro_batches: int = 4,
) -> dict[str, types.ModuleType]:
    training_mod = types.ModuleType("megatron.training.training")
    train_calls: list[int] = []

    def train_step(*args, **kwargs):
        train_calls.append(1)
        return {"loss": 1.0}

    training_mod.train_step = train_step
    training_mod._probing_train_calls = train_calls  # type: ignore[attr-defined]

    global_vars = types.ModuleType("megatron.training.global_vars")
    args_obj = SimpleNamespace(iteration=iteration)
    global_vars.get_args = lambda: args_obj
    global_vars._probing_args = args_obj  # type: ignore[attr-defined]

    num_calc = types.ModuleType("megatron.core.num_microbatches_calculator")
    num_calc.get_num_microbatches = lambda: micro_batches

    core = types.ModuleType("megatron.core")
    core.parallel_state = ps
    core.num_microbatches_calculator = num_calc
    training_pkg = types.ModuleType("megatron.training")
    training_pkg.training = training_mod
    training_pkg.global_vars = global_vars
    megatron = types.ModuleType("megatron")
    megatron.core = core
    megatron.training = training_pkg

    return {
        "megatron": megatron,
        "megatron.core": core,
        "megatron.core.parallel_state": ps,
        "megatron.core.num_microbatches_calculator": num_calc,
        "megatron.training": training_pkg,
        "megatron.training.training": training_mod,
        "megatron.training.global_vars": global_vars,
    }


@contextmanager
def megatron_modules(modules: dict[str, types.ModuleType]):
    with mock.patch.dict(sys.modules, modules):
        yield modules

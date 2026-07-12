"""Megatron-LM / Megatron-Core autostart integration for probing.

When ``PROBING`` is active, probing registers import hooks on Megatron modules and:

* syncs ``probing.set_role`` from ``megatron.core.parallel_state`` after init
* wraps ``train_step`` to align ``probing.step`` with Megatron iteration / microbatches

All hooks are best-effort and no-op when Megatron is absent or APIs differ across versions.
"""

from __future__ import annotations

import functools
import logging
import os
import sys
from typing import Any, Callable, Optional

import probing

from probing.util.env import FALSE_VALUES, TRUE_VALUES, parse_bool_flag

logger = logging.getLogger(__name__)

_PARALLEL_STATE_INIT = False
_TRAINING_INIT = False
_LAST_ROLE: Optional[str] = None
_LAST_ITERATION: Optional[int] = None

# Megatron-style env vars that indicate a Megatron job even before import.
_MEGATRON_ENV_MARKERS = (
    "TENSOR_MODEL_PARALLEL_RANK",
    "TENSOR_MODEL_PARALLEL_SIZE",
    "PIPELINE_MODEL_PARALLEL_RANK",
    "PIPELINE_MODEL_PARALLEL_SIZE",
    "DATA_PARALLEL_RANK",
    "DATA_PARALLEL_SIZE",
    "MEGATRON_CORE_VERSION",
)


def _config_flag(name: str) -> Optional[bool]:
    return parse_bool_flag(probing.config.get_str(name))


def megatron_job_detected() -> bool:
    if any(os.environ.get(key) for key in _MEGATRON_ENV_MARKERS):
        return True
    return any(
        name == "megatron" or name.startswith("megatron.") for name in sys.modules
    )


def megatron_autostart_enabled() -> bool:
    explicit = _config_flag("probing.megatron.enable")
    if explicit is not None:
        return explicit
    raw = os.environ.get("PROBING_MEGATRON", "auto").strip().lower()
    if raw in FALSE_VALUES:
        return False
    if raw in TRUE_VALUES or raw == "on":
        return True
    return megatron_job_detected()


def step_sync_enabled() -> bool:
    explicit = _config_flag("probing.megatron.step_sync")
    if explicit is not None:
        return explicit
    raw = os.environ.get("PROBING_MEGATRON_STEP_SYNC", "auto").strip().lower()
    if raw in FALSE_VALUES:
        return False
    if raw in TRUE_VALUES or raw == "on":
        return True
    return megatron_autostart_enabled()


def _safe_int(value: Any) -> Optional[int]:
    if value is None:
        return None
    try:
        parsed = int(value)
    except (TypeError, ValueError):
        return None
    return parsed if parsed >= 0 else None


def _call_rank_getter(obj: Any, names: tuple[str, ...]) -> Optional[int]:
    for name in names:
        fn = getattr(obj, name, None)
        if not callable(fn):
            continue
        try:
            value = _safe_int(fn())
            if value is not None:
                return value
        except Exception:
            continue
    return None


def _parallel_state_initialized(ps: Any) -> bool:
    for name in (
        "model_parallel_is_initialized",
        "is_initialized",
    ):
        fn = getattr(ps, name, None)
        if callable(fn):
            try:
                return bool(fn())
            except Exception:
                continue
    # Some ranks initialize lazily; attempt rank reads anyway.
    return (
        _call_rank_getter(
            ps,
            ("get_tensor_model_parallel_rank", "get_tensor_model_parallel_world_rank"),
        )
        is not None
    )


def role_dims_from_parallel_state(ps: Any) -> dict[str, int]:
    if not _parallel_state_initialized(ps):
        return {}

    dims: dict[str, int] = {}
    mapping = (
        ("tp", ("get_tensor_model_parallel_rank",)),
        ("pp", ("get_pipeline_model_parallel_rank",)),
        ("dp", ("get_data_parallel_rank",)),
        ("ep", ("get_expert_model_parallel_rank",)),
        ("cp", ("get_context_parallel_rank",)),
    )
    for name, getters in mapping:
        value = _call_rank_getter(ps, getters)
        if value is not None:
            dims[name] = value
    return dims


def sync_role_from_parallel_state(ps: Any | None = None) -> Optional[str]:
    """Read Megatron parallel ranks and push them into ``probing.set_role``."""
    global _LAST_ROLE
    if ps is None:
        try:
            from megatron.core import parallel_state as ps  # type: ignore
        except ImportError:
            return None

    dims = role_dims_from_parallel_state(ps)
    if not dims:
        return None

    role = probing.set_role(**dims)
    if role and role != _LAST_ROLE:
        logger.info("Megatron parallel role synced: %s", role)
        _LAST_ROLE = role
    return role


def _read_megatron_iteration() -> Optional[int]:
    try:
        from megatron.training import global_vars
    except ImportError:
        return None

    args = None
    get_args = getattr(global_vars, "get_args", None)
    if callable(get_args):
        try:
            args = get_args()
        except Exception:
            args = None

    if args is not None:
        for attr in ("iteration", "curr_iteration", "train_iters"):
            value = _safe_int(getattr(args, attr, None))
            if value is not None:
                return value
    return None


def _read_num_microbatches() -> Optional[int]:
    try:
        from megatron.core.num_microbatches_calculator import (
            get_num_microbatches,  # type: ignore
        )
    except ImportError:
        get_num_microbatches = None

    if callable(get_num_microbatches):
        try:
            return _safe_int(get_num_microbatches())
        except Exception:
            pass

    try:
        from megatron.training import global_vars

        get_args = getattr(global_vars, "get_args", None)
        if callable(get_args):
            args = get_args()
            for attr in ("global_batch_size", "micro_batch_size"):
                if not hasattr(args, attr):
                    continue
            gbs = _safe_int(getattr(args, "global_batch_size", None))
            mbs = _safe_int(getattr(args, "micro_batch_size", None))
            if gbs is not None and mbs is not None and mbs > 0:
                return max(1, gbs // mbs)
    except Exception:
        pass
    return None


def sync_step_from_megatron(*, force: bool = False) -> None:
    """Align probing step coordinates with Megatron iteration when available."""
    global _LAST_ITERATION
    if not step_sync_enabled():
        return

    micro_batches = _read_num_microbatches()
    if micro_batches is not None:
        probing.step(micro_batches=micro_batches)
    else:
        micro_batches = int(probing.step.snapshot().micro_batches) or 1

    iteration = _read_megatron_iteration()
    if iteration is None:
        return
    if not force and iteration == _LAST_ITERATION:
        return

    # Megatron ``iteration`` is an optimizer step (probing local_step), not micro_step.
    probing.step(iteration * micro_batches)
    _LAST_ITERATION = iteration


def sync_step_from_iteration(
    iteration: int,
    *,
    micro_batches: Optional[int] = None,
    force: bool = False,
) -> None:
    """Align probing step with a Megatron-Core style optimizer iteration counter."""
    global _LAST_ITERATION
    if not step_sync_enabled():
        return

    value = _safe_int(iteration)
    if value is None:
        return

    mb = micro_batches
    if mb is None:
        mb = _read_num_microbatches()
    if mb is not None:
        probing.step(micro_batches=mb)
    else:
        mb = int(probing.step.snapshot().micro_batches) or 1

    if not force and value == _LAST_ITERATION:
        return

    probing.step(value * mb)
    _LAST_ITERATION = value


def _wrap_callable(module: Any, attr: str, wrapper_builder: Callable) -> None:
    original = getattr(module, attr, None)
    if original is None or getattr(original, "_probing_wrapped", False):
        return

    wrapped = wrapper_builder(original)
    wrapped._probing_wrapped = True  # type: ignore[attr-defined]
    setattr(module, attr, wrapped)


def _wrap_initialize_model_parallel(ps: Any) -> None:
    def builder(original):
        @functools.wraps(original)
        def wrapped(*args, **kwargs):
            result = original(*args, **kwargs)
            sync_role_from_parallel_state(ps)
            return result

        return wrapped

    _wrap_callable(ps, "initialize_model_parallel", builder)


def init_parallel_state() -> None:
    """Import-hook entry when ``megatron.core.parallel_state`` loads."""
    global _PARALLEL_STATE_INIT
    if _PARALLEL_STATE_INIT or not megatron_autostart_enabled():
        return
    _PARALLEL_STATE_INIT = True

    try:
        from megatron.core import parallel_state as ps  # type: ignore
    except ImportError:
        return

    sync_role_from_parallel_state(ps)
    _wrap_initialize_model_parallel(ps)


def _wrap_train_step(training_mod: Any) -> None:
    def builder(original):
        @functools.wraps(original)
        def wrapped(*args, **kwargs):
            sync_step_from_megatron(force=True)
            sync_role_from_parallel_state()
            return original(*args, **kwargs)

        return wrapped

    _wrap_callable(training_mod, "train_step", builder)


def init_training() -> None:
    """Import-hook entry when ``megatron.training.training`` loads."""
    global _TRAINING_INIT
    if _TRAINING_INIT or not megatron_autostart_enabled():
        return
    _TRAINING_INIT = True

    try:
        import megatron.training.training as training_mod  # type: ignore
    except ImportError:
        return

    if step_sync_enabled():
        _wrap_train_step(training_mod)
    sync_step_from_megatron(force=True)
    sync_role_from_parallel_state()


def maybe_autostart() -> None:
    """Best-effort autostart when Megatron was imported before probing hooks."""
    if not megatron_autostart_enabled():
        return

    if "megatron.core.parallel_state" in sys.modules:
        init_parallel_state()
    if "megatron.training.training" in sys.modules:
        init_training()
    elif megatron_job_detected():
        sync_role_from_parallel_state()


def init() -> None:
    """Generic init alias — runs pending autostart if modules are already loaded."""
    maybe_autostart()

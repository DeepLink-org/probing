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
import re
import sys
import time
from dataclasses import dataclass
from typing import Any, Callable, Optional

import probing

from probing.core.table import table
from probing.util.env import FALSE_VALUES, TRUE_VALUES, parse_bool_flag

logger = logging.getLogger(__name__)

_PARALLEL_STATE_INIT = False
_TRAINING_INIT = False
_LAST_ROLE: Optional[str] = None
_LAST_ITERATION: Optional[int] = None
_RECORDED_RANK_LAYOUTS: set[tuple[Any, ...]] = set()
_CAPTURED_MODEL_IDS: set[int] = set()


@table("megatron_rank_layout", capacity_bytes=2 * 1024 * 1024)
@dataclass
class MegatronRankLayout:
    """Static Megatron parallel placement and layer ownership for one model chunk."""

    recorded_at_ns: int
    rank: int
    world_size: int
    local_rank: int
    role: str
    tp_rank: int
    tp_size: int
    pp_rank: int
    pp_size: int
    dp_rank: int
    dp_size: int
    cp_rank: int
    cp_size: int
    ep_rank: int
    ep_size: int
    vp_rank: int
    vp_size: int
    model_chunk: int
    layer_start: int
    layer_end: int
    module_count: int
    layout_key: str
    is_first_pp_stage: int
    is_last_pp_stage: int


@table("megatron_module_catalog", capacity_bytes=16 * 1024 * 1024)
@dataclass
class MegatronModuleCatalog:
    """Static module tree reported by one Megatron rank and model chunk."""

    recorded_at_ns: int
    rank: int
    layout_key: str
    model_chunk: int
    module_path: str
    parent_path: str
    module_type: str
    depth: int
    local_layer_index: int
    global_layer_index: int
    parameter_count: int
    trainable_parameter_count: int
    dtype: str
    device: str
    is_leaf: int

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


def _parallel_value(ps: Any, names: tuple[str, ...], default: int = -1) -> int:
    value = _call_rank_getter(ps, names)
    return value if value is not None else default


def _distributed_value(name: str, env_name: str, default: int = -1) -> int:
    try:
        import torch.distributed as dist

        fn = getattr(dist, name, None)
        if callable(fn) and dist.is_available() and dist.is_initialized():
            return int(fn())
    except Exception:
        pass
    value = _safe_int(os.environ.get(env_name))
    return value if value is not None else default


def _parallel_snapshot(ps: Any) -> dict[str, int]:
    mapping = {
        "tp_rank": ("get_tensor_model_parallel_rank",),
        "tp_size": ("get_tensor_model_parallel_world_size",),
        "pp_rank": ("get_pipeline_model_parallel_rank",),
        "pp_size": ("get_pipeline_model_parallel_world_size",),
        "dp_rank": ("get_data_parallel_rank",),
        "dp_size": ("get_data_parallel_world_size",),
        "cp_rank": ("get_context_parallel_rank",),
        "cp_size": ("get_context_parallel_world_size",),
        "ep_rank": ("get_expert_model_parallel_rank",),
        "ep_size": ("get_expert_model_parallel_world_size",),
        "vp_rank": ("get_virtual_pipeline_model_parallel_rank",),
        "vp_size": ("get_virtual_pipeline_model_parallel_world_size",),
    }
    return {name: _parallel_value(ps, getters) for name, getters in mapping.items()}


def _bool_getter(ps: Any, name: str) -> int:
    fn = getattr(ps, name, None)
    if not callable(fn):
        return -1
    try:
        return int(bool(fn()))
    except Exception:
        return -1


def _layout_key(snapshot: dict[str, int], model_chunk: int) -> str:
    return ",".join(
        (
            f"pp={snapshot['pp_rank']}",
            f"tp={snapshot['tp_rank']}",
            f"ep={snapshot['ep_rank']}",
            f"vp={snapshot['vp_rank']}",
            f"chunk={model_chunk}",
        )
    )


def _save_rank_layout(
    ps: Any,
    *,
    model_chunk: int = -1,
    layer_start: int = -1,
    layer_end: int = -1,
    module_count: int = 0,
    recorded_at_ns: Optional[int] = None,
) -> None:
    snapshot = _parallel_snapshot(ps)
    rank = _distributed_value("get_rank", "RANK")
    world_size = _distributed_value("get_world_size", "WORLD_SIZE")
    local_rank = _safe_int(os.environ.get("LOCAL_RANK"))
    local_rank = local_rank if local_rank is not None else -1
    signature = (
        rank,
        tuple(sorted(snapshot.items())),
        model_chunk,
        layer_start,
        layer_end,
        module_count,
    )
    if signature in _RECORDED_RANK_LAYOUTS:
        return
    _RECORDED_RANK_LAYOUTS.add(signature)
    try:
        MegatronRankLayout.append(
            MegatronRankLayout(
                recorded_at_ns=recorded_at_ns or time.time_ns(),
                rank=rank,
                world_size=world_size,
                local_rank=local_rank,
                role=probing.current_role(),
                model_chunk=model_chunk,
                layer_start=layer_start,
                layer_end=layer_end,
                module_count=module_count,
                layout_key=_layout_key(snapshot, model_chunk),
                is_first_pp_stage=_bool_getter(ps, "is_pipeline_first_stage"),
                is_last_pp_stage=_bool_getter(ps, "is_pipeline_last_stage"),
                **snapshot,
            )
        )
    except Exception as exc:
        _RECORDED_RANK_LAYOUTS.discard(signature)
        logger.debug("Megatron rank layout capture skipped: %s", exc)


_LAYER_PATH_RE = re.compile(r"(?:^|\.)(?:layers?|transformer_layers)\.(\d+)(?:\.|$)")


def _local_layer_index(path: str) -> int:
    match = _LAYER_PATH_RE.search(path)
    return int(match.group(1)) if match else -1


def _global_layer_index(module: Any) -> int:
    for name in ("global_layer_number", "layer_number"):
        value = _safe_int(getattr(module, name, None))
        if value is not None:
            # Megatron TransformerLayer.layer_number is conventionally one-based.
            return max(0, value - 1) if name == "layer_number" else value
    return -1


def _direct_parameters(module: Any) -> list[Any]:
    fn = getattr(module, "parameters", None)
    if not callable(fn):
        return []
    try:
        return list(fn(recurse=False))
    except TypeError:
        return []
    except Exception:
        return []


def _parameter_count(parameters: list[Any], *, trainable_only: bool = False) -> int:
    total = 0
    for parameter in parameters:
        if trainable_only and not bool(getattr(parameter, "requires_grad", False)):
            continue
        try:
            total += int(parameter.numel())
        except Exception:
            continue
    return total


def _module_records(
    model: Any,
    *,
    rank: int,
    layout_key: str,
    model_chunk: int,
    recorded_at_ns: int,
) -> list[MegatronModuleCatalog]:
    named_modules = getattr(model, "named_modules", None)
    if not callable(named_modules):
        return []
    try:
        modules = list(named_modules())
    except Exception:
        return []

    global_by_local: dict[int, int] = {}
    for path, module in modules:
        local = _local_layer_index(str(path))
        global_index = _global_layer_index(module)
        if local >= 0 and global_index >= 0:
            global_by_local.setdefault(local, global_index)

    records = []
    for path, module in modules:
        path = str(path)
        local = _local_layer_index(path)
        global_index = _global_layer_index(module)
        if global_index < 0 and local >= 0:
            global_index = global_by_local.get(local, -1)
        parameters = _direct_parameters(module)
        first_parameter = parameters[0] if parameters else None
        try:
            is_leaf = int(not any(True for _ in module.children()))
        except Exception:
            is_leaf = -1
        records.append(
            MegatronModuleCatalog(
                recorded_at_ns=recorded_at_ns,
                rank=rank,
                layout_key=layout_key,
                model_chunk=model_chunk,
                module_path=path or "<root>",
                parent_path=path.rsplit(".", 1)[0] if "." in path else "",
                module_type=f"{type(module).__module__}.{type(module).__qualname__}",
                depth=0 if not path else path.count(".") + 1,
                local_layer_index=local,
                global_layer_index=global_index,
                parameter_count=_parameter_count(parameters),
                trainable_parameter_count=_parameter_count(
                    parameters, trainable_only=True
                ),
                dtype=str(getattr(first_parameter, "dtype", "")),
                device=str(getattr(first_parameter, "device", "")),
                is_leaf=is_leaf,
            )
        )
    return records


def capture_model_layout(models: Any, ps: Any | None = None) -> None:
    """Persist a one-time static module catalog after Megatron builds the model."""
    if ps is None:
        try:
            from megatron.core import parallel_state as ps  # type: ignore
        except ImportError:
            return
    chunks = list(models) if isinstance(models, (list, tuple)) else [models]
    rank = _distributed_value("get_rank", "RANK")
    snapshot = _parallel_snapshot(ps)
    recorded_at_ns = time.time_ns()
    for chunk_index, model in enumerate(chunks):
        identity = id(model)
        if identity in _CAPTURED_MODEL_IDS:
            continue
        _CAPTURED_MODEL_IDS.add(identity)
        key = _layout_key(snapshot, chunk_index)
        records = _module_records(
            model,
            rank=rank,
            layout_key=key,
            model_chunk=chunk_index,
            recorded_at_ns=recorded_at_ns,
        )
        if records:
            try:
                MegatronModuleCatalog.append_many(records)
            except Exception as exc:
                _CAPTURED_MODEL_IDS.discard(identity)
                logger.debug("Megatron module catalog capture skipped: %s", exc)
                continue
        layer_indices = [
            record.global_layer_index
            for record in records
            if record.global_layer_index >= 0
        ]
        _save_rank_layout(
            ps,
            model_chunk=chunk_index,
            layer_start=min(layer_indices, default=-1),
            layer_end=max(layer_indices, default=-1),
            module_count=len(records),
            recorded_at_ns=recorded_at_ns,
        )


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
    _save_rank_layout(ps)
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


def _wrap_get_model(training_mod: Any) -> None:
    def builder(original):
        @functools.wraps(original)
        def wrapped(*args, **kwargs):
            models = original(*args, **kwargs)
            try:
                capture_model_layout(models)
            except Exception as exc:
                logger.debug("Megatron model layout capture skipped: %s", exc)
            return models

        return wrapped

    _wrap_callable(training_mod, "get_model", builder)


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
    _wrap_get_model(training_mod)
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

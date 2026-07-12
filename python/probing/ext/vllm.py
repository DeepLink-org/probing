"""vLLM / vLLM-Metal autostart integration for probing.

When ``PROBING`` is active, probing registers import hooks on vLLM entry points and:

* syncs ``probing.set_role`` from distributed env and tags ``backend=metal`` when
  the macOS ``vllm-metal`` platform plugin is present
* wraps ``LLMEngine.step`` to align ``probing.step`` with scheduler iterations

All hooks are best-effort and no-op when vLLM is absent or APIs differ across versions.
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

_METAL_INIT = False
_ENGINE_INIT_DONE: set[str] = set()
_LAST_ROLE: Optional[str] = None
_LAST_STEP: Optional[int] = None

# vLLM-style env vars that indicate a vLLM job even before import.
_VLLM_ENV_MARKERS = (
    "VLLM_TARGET_DEVICE",
    "VLLM_USE_V1",
    "VLLM_WORKER_MULTIPROC_METHOD",
    "VLLM_HOST_IP",
    "VLLM_PORT",
    "VLLM_LOGGING_LEVEL",
    "VLLM_ATTENTION_BACKEND",
)

# vLLM-Metal (macOS Apple Silicon) plugin markers.
_METAL_ENV_MARKERS = (
    "VLLM_METAL_USE_MLX",
    "VLLM_MLX_DEVICE",
    "VLLM_METAL_MEMORY_FRACTION",
    "VLLM_METAL_BLOCK_SIZE",
)

_ENGINE_MODULE_PATHS = (
    "vllm.v1.engine.llm_engine",
    "vllm.engine.llm_engine",
)
_ENGINE_CLASS = "LLMEngine"
_ENGINE_STEP_METHOD = "step"


def _config_flag(name: str) -> Optional[bool]:
    return parse_bool_flag(probing.config.get_str(name))


def metal_backend_detected() -> bool:
    if "vllm_metal" in sys.modules:
        return True
    return any(os.environ.get(key) for key in _METAL_ENV_MARKERS)


def vllm_job_detected() -> bool:
    if any(os.environ.get(key) for key in _VLLM_ENV_MARKERS):
        return True
    if metal_backend_detected():
        return True
    return any(name == "vllm" or name.startswith("vllm.") for name in sys.modules)


def vllm_autostart_enabled() -> bool:
    explicit = _config_flag("probing.vllm.enable")
    if explicit is not None:
        return explicit
    raw = os.environ.get("PROBING_VLLM", "auto").strip().lower()
    if raw in FALSE_VALUES:
        return False
    if raw in TRUE_VALUES or raw == "on":
        return True
    return vllm_job_detected()


def step_sync_enabled() -> bool:
    explicit = _config_flag("probing.vllm.step_sync")
    if explicit is not None:
        return explicit
    raw = os.environ.get("PROBING_VLLM_STEP_SYNC", "auto").strip().lower()
    if raw in FALSE_VALUES:
        return False
    if raw in TRUE_VALUES or raw == "on":
        return True
    return vllm_autostart_enabled()


def _safe_int(value: Any) -> Optional[int]:
    if value is None:
        return None
    try:
        parsed = int(value)
    except (TypeError, ValueError):
        return None
    return parsed if parsed >= 0 else None


def sync_role_from_env() -> Optional[str]:
    """Read vLLM / torchrun env and push parallel role into ``probing.set_role``."""
    global _LAST_ROLE
    dims: dict[str, Any] = {}

    rank = _safe_int(os.environ.get("RANK") or os.environ.get("LOCAL_RANK"))
    if rank is not None:
        dims["rank"] = rank

    for name, keys in (
        ("tp", ("TENSOR_MODEL_PARALLEL_RANK", "TP_RANK")),
        ("pp", ("PIPELINE_MODEL_PARALLEL_RANK", "PP_RANK")),
        ("dp", ("DATA_PARALLEL_RANK", "DP_RANK")),
    ):
        for key in keys:
            value = _safe_int(os.environ.get(key))
            if value is not None:
                dims[name] = value
                break

    if metal_backend_detected():
        dims["backend"] = "metal"
        mlx_device = os.environ.get("VLLM_MLX_DEVICE", "").strip()
        if mlx_device:
            dims["mlx"] = mlx_device

    if not dims:
        return None

    role = probing.set_role(dims)
    if role and role != _LAST_ROLE:
        logger.info("vLLM parallel role synced: %s", role)
        _LAST_ROLE = role
    return role


def _read_engine_step_counter(engine: Any) -> Optional[int]:
    for attr in (
        "step_counter",
        "_step_counter",
        "current_iteration",
        "iteration",
    ):
        value = _safe_int(getattr(engine, attr, None))
        if value is not None:
            return value
    return None


def sync_step_from_engine(engine: Any | None = None, *, force: bool = False) -> None:
    """Align probing step coordinates with vLLM engine scheduler steps."""
    global _LAST_STEP
    if not step_sync_enabled():
        return

    if engine is None:
        probing.step()
        return

    counter = _read_engine_step_counter(engine)
    if counter is None:
        probing.step()
        return
    if not force and counter == _LAST_STEP:
        return

    probing.step(counter)
    _LAST_STEP = counter


def sync_step_from_llm(llm: Any | None = None, *, force: bool = False) -> None:
    """Best-effort step sync from an offline ``vLLM.LLM`` instance."""
    if llm is None:
        sync_step_from_engine(None, force=force)
        return
    for attr in ("llm_engine", "engine", "engine_core"):
        engine = getattr(llm, attr, None)
        if engine is not None:
            sync_step_from_engine(engine, force=force)
            return
    sync_step_from_engine(None, force=force)


def _wrap_callable(owner: Any, attr: str, wrapper_builder: Callable) -> None:
    original = getattr(owner, attr, None)
    if original is None or getattr(original, "_probing_wrapped", False):
        return

    wrapped = wrapper_builder(original)
    wrapped._probing_wrapped = True  # type: ignore[attr-defined]
    setattr(owner, attr, wrapped)


def _wrap_engine_step(engine_cls: Any) -> None:
    def builder(original):
        @functools.wraps(original)
        def wrapped(self, *args, **kwargs):
            result = original(self, *args, **kwargs)
            sync_step_from_engine(self, force=True)
            sync_role_from_env()
            return result

        return wrapped

    _wrap_callable(engine_cls, _ENGINE_STEP_METHOD, builder)


def init_metal_platform() -> None:
    """Import-hook entry when ``vllm_metal`` loads (macOS Metal plugin)."""
    global _METAL_INIT
    if _METAL_INIT or not vllm_autostart_enabled():
        return
    _METAL_INIT = True
    sync_role_from_env()


def init_engine(module_path: str) -> None:
    """Import-hook entry when a vLLM ``LLMEngine`` module loads."""
    if module_path in _ENGINE_INIT_DONE or not vllm_autostart_enabled():
        return
    _ENGINE_INIT_DONE.add(module_path)

    mod = sys.modules.get(module_path)
    if mod is None:
        return

    engine_cls = getattr(mod, _ENGINE_CLASS, None)
    if engine_cls is None:
        return

    sync_role_from_env()
    if step_sync_enabled():
        _wrap_engine_step(engine_cls)


def init_v1_engine() -> None:
    init_engine("vllm.v1.engine.llm_engine")


def init_classic_engine() -> None:
    init_engine("vllm.engine.llm_engine")


def maybe_autostart() -> None:
    """Best-effort autostart when vLLM was imported before probing hooks."""
    if not vllm_autostart_enabled():
        return

    if "vllm_metal" in sys.modules:
        init_metal_platform()
    for module_path in _ENGINE_MODULE_PATHS:
        if module_path in sys.modules:
            init_engine(module_path)
    if vllm_job_detected():
        sync_role_from_env()


def init() -> None:
    """Generic init alias — runs pending autostart if modules are already loaded."""
    maybe_autostart()

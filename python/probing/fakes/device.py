"""Remap CUDA device APIs onto ``meta`` / ``cpu`` / ``mps`` for macOS debugging."""

from __future__ import annotations

import logging
from dataclasses import dataclass
from typing import Any, Callable, Optional

logger = logging.getLogger(__name__)

_SUPPORTED = frozenset({"meta", "cpu", "mps"})
_FACTORY_NAMES = (
    "empty",
    "zeros",
    "ones",
    "randn",
    "rand",
    "full",
    "arange",
    "tensor",
    "as_tensor",
    "empty_like",
    "zeros_like",
    "ones_like",
    "randn_like",
    "rand_like",
    "full_like",
)


@dataclass
class _DevicePatchState:
    target: str = "meta"
    installed: bool = False
    originals: dict[str, Any] | None = None


_STATE = _DevicePatchState()


def resolve_fake_device(raw: str | None = None) -> str:
    import os

    text = (
        raw if raw is not None else os.environ.get("PROBING_FAKE_DEVICE", "meta")
    ).strip()
    lowered = text.lower() or "meta"
    if lowered not in _SUPPORTED:
        raise ValueError(
            f"PROBING_FAKE_DEVICE={text!r} unsupported; use one of {sorted(_SUPPORTED)}"
        )
    if lowered == "mps":
        import torch

        mps = getattr(torch.backends, "mps", None)
        if mps is None or not mps.is_available():
            logger.warning("mps unavailable; falling back to cpu for probing.fakes")
            return "cpu"
    return lowered


def target_device() -> str:
    return _STATE.target


def is_device_remap_installed() -> bool:
    return _STATE.installed


def _is_cuda_spec(value: Any, *, torch: Any) -> bool:
    if isinstance(value, torch.device):
        return value.type == "cuda"
    # ``torch.cuda.current_device()`` returns an int index; with remap installed
    # ``is_available()`` is True so factories treat bare ints as CUDA ordinals.
    if isinstance(value, int):
        return True
    text = str(value).strip().lower()
    return text == "cuda" or text.startswith("cuda:")


def _as_target_device(device: Any, *, torch: Any) -> Any:
    target = _STATE.target
    if device is None:
        return torch.device(target)
    if _is_cuda_spec(device, torch=torch):
        return torch.device(target)
    return device


def _wrap_factory(orig: Callable[..., Any], torch: Any) -> Callable[..., Any]:
    def wrapped(*args: Any, **kwargs: Any) -> Any:
        if "device" in kwargs and _is_cuda_spec(kwargs["device"], torch=torch):
            kwargs = {**kwargs, "device": _STATE.target}
        return orig(*args, **kwargs)

    return wrapped


class _FakeCudaStream:
    """No-op stand-in for ``torch.cuda.Stream`` on non-CUDA hosts."""

    def __init__(self, device: Any = None, priority: int = 0, **_kwargs: Any) -> None:
        del priority
        import torch

        if device is None:
            self.device = torch.device(_STATE.target)
        elif isinstance(device, int):
            self.device = torch.device(_STATE.target)
        else:
            self.device = (
                torch.device(device) if not isinstance(device, torch.device) else device
            )
        self.stream_id = 0
        self.device_index = 0
        self.device_type = getattr(self.device, "type", _STATE.target)

    def wait_stream(self, _stream: Any) -> None:
        return None

    def wait_event(self, _event: Any) -> None:
        return None

    def record_event(self, event: Any = None) -> Any:
        return event if event is not None else _FakeCudaEvent()

    def synchronize(self) -> None:
        return None

    def query(self) -> bool:
        return True

    def __eq__(self, other: object) -> bool:
        return isinstance(other, _FakeCudaStream)

    def __hash__(self) -> int:
        return id(self)

    def __enter__(self) -> "_FakeCudaStream":
        return self

    def __exit__(self, *exc: Any) -> None:
        return None


class _FakeCudaEvent:
    def __init__(
        self,
        enable_timing: bool = False,
        blocking: bool = False,
        interprocess: bool = False,
    ) -> None:
        del enable_timing, blocking, interprocess

    def record(self, _stream: Any = None) -> None:
        return None

    def wait(self, _stream: Any = None) -> None:
        return None

    def query(self) -> bool:
        return True

    def synchronize(self) -> None:
        return None

    def elapsed_time(self, _end_event: Any) -> float:
        return 0.0


_FAKE_CURRENT_STREAM = _FakeCudaStream()


def install_device_remap(device: str | None = None) -> str:
    """Patch ``torch.cuda`` / ``Tensor.cuda`` / ``Module.cuda`` onto ``device``."""
    import torch

    target = resolve_fake_device(device)
    if _STATE.installed:
        _STATE.target = target
        return target

    originals: dict[str, Any] = {}

    cuda_mod = torch.cuda
    for name in (
        "is_available",
        "device_count",
        "current_device",
        "set_device",
        "synchronize",
        "empty_cache",
        "is_initialized",
        "manual_seed",
        "manual_seed_all",
    ):
        originals[name] = getattr(cuda_mod, name, None)

    def _is_available() -> bool:
        return True

    def _device_count() -> int:
        return 1

    def _current_device() -> int:
        return 0

    def _set_device(_idx: int = 0) -> None:
        return None

    def _synchronize(_device: Any = None) -> None:
        return None

    def _empty_cache() -> None:
        return None

    def _is_initialized() -> bool:
        return True

    def _manual_seed(_seed: int) -> None:
        # No-op: torch.manual_seed() itself calls cuda.manual_seed_all when
        # is_available() is True — must not recurse into torch.manual_seed.
        return None

    def _manual_seed_all(_seed: int) -> None:
        return None

    def _get_rng_state(device: Any = None) -> Any:
        # Mirror CPU RNG so Megatron's CudaRNGStatesTracker can fork/restore.
        del device
        return torch.get_rng_state()

    def _set_rng_state(new_state: Any, device: Any = None) -> None:
        del device
        torch.set_rng_state(new_state)

    def _get_rng_state_all() -> list[Any]:
        return [_get_rng_state()]

    def _set_rng_state_all(states: list[Any]) -> None:
        if states:
            _set_rng_state(states[0])

    def _lazy_init() -> None:
        return None

    def _lazy_call(callable_fn: Any, **_kwargs: Any) -> None:
        # Host CUDA queues these until real init; with fake is_available we run now.
        try:
            callable_fn()
        except Exception as exc:
            logger.debug("cuda._lazy_call suppressed: %s", exc)

    # Always overwrite — host CUDA stubs exist on macOS but crash when called.
    cuda_mod.is_available = _is_available  # type: ignore[method-assign]
    cuda_mod.device_count = _device_count  # type: ignore[method-assign]
    cuda_mod.current_device = _current_device  # type: ignore[method-assign]
    cuda_mod.set_device = _set_device  # type: ignore[method-assign]
    cuda_mod.synchronize = _synchronize  # type: ignore[method-assign]
    cuda_mod.empty_cache = _empty_cache  # type: ignore[method-assign]
    cuda_mod.is_initialized = _is_initialized  # type: ignore[method-assign]
    cuda_mod.manual_seed = _manual_seed  # type: ignore[method-assign]
    cuda_mod.manual_seed_all = _manual_seed_all  # type: ignore[method-assign]

    for rng_name, rng_fn in (
        ("get_rng_state", _get_rng_state),
        ("set_rng_state", _set_rng_state),
        ("get_rng_state_all", _get_rng_state_all),
        ("set_rng_state_all", _set_rng_state_all),
    ):
        originals[rng_name] = getattr(cuda_mod, rng_name, None)
        setattr(cuda_mod, rng_name, rng_fn)

    originals["_lazy_init"] = getattr(cuda_mod, "_lazy_init", None)
    cuda_mod._lazy_init = _lazy_init  # type: ignore[method-assign]
    originals["_lazy_call"] = getattr(cuda_mod, "_lazy_call", None)
    cuda_mod._lazy_call = _lazy_call  # type: ignore[method-assign]

    # Megatron's CudaRNGStatesTracker indexes default_generators[current_device].
    originals["default_generators"] = getattr(cuda_mod, "default_generators", ())
    try:
        cuda_mod.default_generators = (torch.Generator(device="cpu"),)  # type: ignore[misc]
    except Exception as exc:
        logger.debug("default_generators stub skipped: %s", exc)

    # Stream / Event are dummy base classes on non-CUDA builds.
    originals["Stream"] = getattr(cuda_mod, "Stream", None)
    originals["Event"] = getattr(cuda_mod, "Event", None)
    originals["current_stream"] = getattr(cuda_mod, "current_stream", None)
    originals["default_stream"] = getattr(cuda_mod, "default_stream", None)
    cuda_mod.Stream = _FakeCudaStream  # type: ignore[misc, assignment]
    cuda_mod.Event = _FakeCudaEvent  # type: ignore[misc, assignment]

    def _current_stream(_device: Any = None) -> _FakeCudaStream:
        return _FAKE_CURRENT_STREAM

    def _default_stream(_device: Any = None) -> _FakeCudaStream:
        return _FAKE_CURRENT_STREAM

    cuda_mod.current_stream = _current_stream  # type: ignore[method-assign]
    if originals["default_stream"] is not None:
        cuda_mod.default_stream = _default_stream  # type: ignore[method-assign]

    class _NoopCtx:
        def __init__(self, *_a: Any, **_k: Any) -> None:
            return None

        def __enter__(self) -> None:
            return None

        def __exit__(self, *exc: Any) -> bool:
            return False

    def _stream_ctx(_stream: Any = None) -> _NoopCtx:
        return _NoopCtx()

    def _device_ctx(_device: Any = None) -> _NoopCtx:
        return _NoopCtx()

    def _set_stream(_stream: Any = None) -> None:
        return None

    originals["stream"] = getattr(cuda_mod, "stream", None)
    originals["device_ctx"] = getattr(cuda_mod, "device", None)
    originals["set_stream"] = getattr(cuda_mod, "set_stream", None)
    cuda_mod.stream = _stream_ctx  # type: ignore[method-assign, assignment]
    cuda_mod.device = _device_ctx  # type: ignore[misc, assignment]
    if originals["set_stream"] is not None:
        cuda_mod.set_stream = _set_stream  # type: ignore[method-assign]

    # torch.cuda.random binds current_device/device_count at import time — refresh.
    random_mod = getattr(cuda_mod, "random", None)
    if random_mod is not None:
        for name, fn in (
            ("current_device", _current_device),
            ("device_count", _device_count),
            ("get_rng_state", _get_rng_state),
            ("set_rng_state", _set_rng_state),
            ("get_rng_state_all", _get_rng_state_all),
            ("set_rng_state_all", _set_rng_state_all),
            ("manual_seed", _manual_seed),
            ("manual_seed_all", _manual_seed_all),
        ):
            key = f"random.{name}"
            originals[key] = getattr(random_mod, name, None)
            setattr(random_mod, name, fn)

    originals["Tensor.cuda"] = torch.Tensor.cuda
    originals["Tensor.to"] = torch.Tensor.to
    originals["Module.cuda"] = torch.nn.Module.cuda
    originals["Module.to"] = torch.nn.Module.to

    _orig_tensor_to = originals["Tensor.to"]
    _orig_module_to = originals["Module.to"]

    def _tensor_cuda(self: Any, device: Any = None, *args: Any, **kwargs: Any) -> Any:
        del device, args, kwargs
        return _orig_tensor_to(self, target)

    def _module_cuda(self: Any, device: Any = None) -> Any:
        del device
        return _orig_module_to(self, target)

    def _tensor_to(self: Any, *args: Any, **kwargs: Any) -> Any:
        if args and _is_cuda_spec(args[0], torch=torch):
            args = (_as_target_device(args[0], torch=torch),) + args[1:]
        if "device" in kwargs and _is_cuda_spec(kwargs["device"], torch=torch):
            kwargs = {
                **kwargs,
                "device": _as_target_device(kwargs["device"], torch=torch),
            }
        return _orig_tensor_to(self, *args, **kwargs)

    def _module_to(self: Any, *args: Any, **kwargs: Any) -> Any:
        if args and not isinstance(args[0], torch.nn.Module):
            if _is_cuda_spec(args[0], torch=torch):
                args = (_as_target_device(args[0], torch=torch),) + args[1:]
        if "device" in kwargs and _is_cuda_spec(kwargs["device"], torch=torch):
            kwargs = {
                **kwargs,
                "device": _as_target_device(kwargs["device"], torch=torch),
            }
        return _orig_module_to(self, *args, **kwargs)

    torch.Tensor.cuda = _tensor_cuda  # type: ignore[method-assign, assignment]
    torch.Tensor.to = _tensor_to  # type: ignore[method-assign, assignment]
    torch.nn.Module.cuda = _module_cuda  # type: ignore[method-assign, assignment]
    torch.nn.Module.to = _module_to  # type: ignore[method-assign, assignment]

    for name in _FACTORY_NAMES:
        orig = getattr(torch, name, None)
        if callable(orig):
            originals[f"torch.{name}"] = orig
            setattr(torch, name, _wrap_factory(orig, torch))

    _STATE.target = target
    _STATE.installed = True
    _STATE.originals = originals
    logger.info("probing.fakes device remap installed (cuda → %s)", target)
    return target


def uninstall_device_remap() -> None:
    if not _STATE.installed or not _STATE.originals:
        return
    import torch

    originals = _STATE.originals
    cuda_mod = torch.cuda
    for name in (
        "is_available",
        "device_count",
        "current_device",
        "set_device",
        "synchronize",
        "empty_cache",
        "is_initialized",
        "manual_seed",
        "manual_seed_all",
        "get_rng_state",
        "set_rng_state",
        "get_rng_state_all",
        "set_rng_state_all",
        "_lazy_init",
        "_lazy_call",
        "default_generators",
        "Stream",
        "Event",
        "current_stream",
        "default_stream",
        "stream",
        "device_ctx",
        "set_stream",
    ):
        if name not in originals:
            continue
        attr = "device" if name == "device_ctx" else name
        setattr(cuda_mod, attr, originals[name])

    random_mod = getattr(cuda_mod, "random", None)
    if random_mod is not None:
        for name in (
            "current_device",
            "device_count",
            "get_rng_state",
            "set_rng_state",
            "get_rng_state_all",
            "set_rng_state_all",
            "manual_seed",
            "manual_seed_all",
        ):
            key = f"random.{name}"
            if key in originals:
                setattr(random_mod, name, originals[key])

    torch.Tensor.cuda = originals["Tensor.cuda"]  # type: ignore[method-assign]
    torch.Tensor.to = originals["Tensor.to"]  # type: ignore[method-assign]
    torch.nn.Module.cuda = originals["Module.cuda"]  # type: ignore[method-assign]
    torch.nn.Module.to = originals["Module.to"]  # type: ignore[method-assign]

    for name in _FACTORY_NAMES:
        key = f"torch.{name}"
        if key in originals:
            setattr(torch, name, originals[key])

    _STATE.installed = False
    _STATE.originals = None
    logger.info("probing.fakes device remap uninstalled")

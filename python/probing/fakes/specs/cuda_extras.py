"""No-op / attribute-synthesizing stubs for CUDA-only third-party packages.

These are the **bottom layer** fakes: real Megatron-LM / megatron-core code stays
in place; only packages that cannot install or run on macOS (triton, TE, apex,
flash-attn) are invented.
"""

from __future__ import annotations

import types
from typing import Any

from ..registry import FakeSpec, register


def _mark(module: types.ModuleType) -> types.ModuleType:
    module.__probing_fake__ = True  # type: ignore[attr-defined]
    return module


class StubModule(types.ModuleType):
    """Package that synthesizes missing attrs (nested modules / callables / types)."""

    _SUBMODULE_NAMES = frozenset(
        {
            "core",
            "language",
            "runtime",
            "ops",
            "pytorch",
            "contrib",
            "testing",
            "compiler",
            "backends",
            "amp",
            "optimizers",
            "transformer",
        }
    )

    def __init__(self, name: str):
        super().__init__(name)
        self.__probing_fake__ = True  # type: ignore[attr-defined]
        self.__path__ = []  # type: ignore[attr-defined]

    def __getattr__(self, name: str) -> Any:
        if name.startswith("__") and name.endswith("__"):
            raise AttributeError(name)
        if name == "__version__":
            ver = "0.0.0+probing.fakes"
            setattr(self, name, ver)
            return ver
        # Nested packages (triton.language.core, te.pytorch, …)
        if name in self._SUBMODULE_NAMES or (
            name.islower() and "_" not in name and name not in {"jit", "autotune"}
        ):
            child_name = f"{self.__name__}.{name}"
            if child_name in _CACHE:
                child = _CACHE[child_name]
            else:
                child = StubModule(child_name)
                _CACHE[child_name] = child
            setattr(self, name, child)
            return child
        if name[:1].isupper() or name == name.upper():
            sentinel = type(
                name,
                (),
                {
                    "__init__": lambda self, *a, **k: None,
                    "__call__": lambda self, *a, **k: self,
                    "__enter__": lambda self: self,
                    "__exit__": lambda self, *a: None,
                },
            )
            setattr(self, name, sentinel)
            return sentinel

        def stub(*args: Any, **kwargs: Any) -> Any:
            del args, kwargs
            return None

        stub.__name__ = name
        stub.__qualname__ = name
        setattr(self, name, stub)
        return stub


_CACHE: dict[str, types.ModuleType] = {}


def _package(fullname: str) -> types.ModuleType:
    if fullname in _CACHE:
        return _CACHE[fullname]
    mod = _mark(StubModule(fullname))
    _CACHE[fullname] = mod
    return mod


def _wire_triton(mod: types.ModuleType) -> None:
    """Enough surface for megatron-core *and* torch._inductor triton_compat."""
    language = StubModule("triton.language")
    core = StubModule("triton.language.core")

    def _view(x, *args, _semantic=None, **kwargs):
        del args, _semantic, kwargs
        return x

    def _reshape(x, *args, _semantic=None, **kwargs):
        del args, _semantic, kwargs
        return x

    core.view = _view  # type: ignore[attr-defined]
    core.reshape = _reshape  # type: ignore[attr-defined]
    language.core = core  # type: ignore[attr-defined]
    language.constexpr = lambda x=None, *a, **k: x  # type: ignore[attr-defined]
    language.tensor = type("tensor", (), {})  # type: ignore[attr-defined]
    language.constexpr_type = language.constexpr  # type: ignore[attr-defined]
    mod.language = language  # type: ignore[attr-defined]
    mod.__version__ = "3.0.0+probing.fakes"  # type: ignore[attr-defined]
    mod.jit = lambda fn=None, **kwargs: (  # type: ignore[attr-defined]
        fn if fn is not None else (lambda f: f)
    )
    mod.autotune = lambda *a, **k: lambda f: f  # type: ignore[attr-defined]
    mod.heuristics = lambda *a, **k: lambda f: f  # type: ignore[attr-defined]
    mod.Config = type("Config", (), {"__init__": lambda self, *a, **k: None})  # type: ignore[attr-defined]
    _CACHE["triton.language"] = language
    _CACHE["triton.language.core"] = core


def _wire_transformer_engine(mod: types.ModuleType) -> None:
    import torch.nn as nn

    class _PassthroughLinear(nn.Linear):
        def __init__(self, *args: Any, **kwargs: Any) -> None:
            # TE Linear has extra kwargs; keep a usable torch Linear when possible.
            in_f = kwargs.get("in_features", args[0] if args else 1)
            out_f = kwargs.get("out_features", args[1] if len(args) > 1 else in_f)
            bias = kwargs.get("bias", True)
            try:
                super().__init__(int(in_f), int(out_f), bias=bool(bias))
            except Exception:
                super().__init__(1, 1)

    pytorch = StubModule("transformer_engine.pytorch")
    pytorch.Linear = _PassthroughLinear  # type: ignore[attr-defined]
    pytorch.LayerNorm = nn.LayerNorm  # type: ignore[attr-defined]
    pytorch.TransformerLayer = nn.Identity  # type: ignore[attr-defined]
    pytorch.fp8_autocast = lambda *a, **k: _NullCtx()  # type: ignore[attr-defined]
    pytorch.is_fp8_available = lambda *a, **k: False  # type: ignore[attr-defined]
    mod.pytorch = pytorch  # type: ignore[attr-defined]
    _CACHE["transformer_engine.pytorch"] = pytorch


class _NullCtx:
    def __enter__(self) -> _NullCtx:
        return self

    def __exit__(self, *args: object) -> None:
        return None


def transformer_engine_factory(fullname: str) -> types.ModuleType:
    mod = _package(fullname)
    if fullname == "transformer_engine":
        _wire_transformer_engine(mod)
    return mod


def apex_factory(fullname: str) -> types.ModuleType:
    mod = _package(fullname)
    if fullname == "apex.optimizers":
        import torch

        class FusedAdam(torch.optim.AdamW):
            """Apex-compatible AdamW that ignores CUDA-only constructor kwargs."""

            def __init__(self, params, *args, **kwargs):
                for key in (
                    "adam_w_mode",
                    "set_grad_none",
                    "capturable",
                    "master_weights",
                    "use_decoupled_grad",
                ):
                    kwargs.pop(key, None)
                # Apex uses `bias_correction`; torch.AdamW uses `amsgrad` etc.
                kwargs.pop("bias_correction", None)
                super().__init__(params, *args, **kwargs)

        class FusedSGD(torch.optim.SGD):
            def __init__(self, params, *args, **kwargs):
                kwargs.pop("set_grad_none", None)
                super().__init__(params, *args, **kwargs)

        mod.FusedAdam = FusedAdam  # type: ignore[attr-defined]
        mod.FusedSGD = FusedSGD  # type: ignore[attr-defined]
    return mod


def flash_attn_factory(fullname: str) -> types.ModuleType:
    mod = _package(fullname)
    if fullname == "flash_attn":
        mod.flash_attn_func = lambda *a, **k: None  # type: ignore[attr-defined]
        mod.flash_attn_varlen_func = lambda *a, **k: None  # type: ignore[attr-defined]
    return mod


def triton_factory(fullname: str) -> types.ModuleType:
    if fullname == "triton.language" and fullname in _CACHE:
        return _CACHE[fullname]
    mod = _package(fullname)
    if fullname == "triton":
        _wire_triton(mod)
    if fullname == "triton.language":
        # Ensure parent got language wired even if language imported first.
        parent = _package("triton")
        if not hasattr(parent, "language"):
            _wire_triton(parent)
        return _CACHE.get("triton.language", mod)
    return mod


register(
    FakeSpec(
        name="transformer_engine",
        prefixes=("transformer_engine",),
        factory=transformer_engine_factory,
        description="No-op Transformer Engine stubs",
    )
)
register(
    FakeSpec(
        name="apex",
        prefixes=("apex",),
        factory=apex_factory,
        description="No-op NVIDIA Apex stubs",
    )
)
register(
    FakeSpec(
        name="flash_attn",
        prefixes=("flash_attn",),
        factory=flash_attn_factory,
        description="No-op FlashAttention stubs",
    )
)
register(
    FakeSpec(
        name="triton",
        prefixes=("triton",),
        factory=triton_factory,
        description="Triton surface stubs so megatron-core can import on macOS",
    )
)

# Bottom-layer default set used when running real Megatron-LM.
BOTTOM_LAYER_SPECS = (
    "triton",
    "transformer_engine",
    "apex",
    "flash_attn",
)

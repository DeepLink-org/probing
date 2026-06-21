"""PyTorch model/optimizer hooks for automatic training phase spans."""

from __future__ import annotations

import logging
from typing import Optional

from probing.tracing.phases import BACKWARD, FORWARD, OPTIMIZER, hook_enter, hook_exit

logger = logging.getLogger(__name__)

_REGISTRY: dict[tuple[int, int], PhaseTracker] = {}


class PhaseTracker:
    def __init__(self, model, optimizer) -> None:
        self.model = model
        self.optimizer = optimizer
        self._handles: list = []

    def install(self) -> None:
        if self._handles:
            return
        m = self.model
        opt = self.optimizer
        self._handles = [
            m.register_forward_pre_hook(self._forward_pre),
            m.register_forward_hook(self._forward_post),
            m.register_full_backward_pre_hook(self._backward_pre),
            m.register_full_backward_hook(self._backward_post),
            opt.register_step_pre_hook(self._step_pre),
            opt.register_step_post_hook(self._step_post),
        ]

    def uninstall(self) -> None:
        for h in self._handles:
            try:
                h.remove()
            except Exception:
                pass
        self._handles.clear()

    def _forward_pre(self, module, _inputs) -> None:
        if module.training:
            hook_enter(FORWARD)

    def _forward_post(self, module, _inputs, _output) -> None:
        if module.training:
            hook_exit(FORWARD)

    def _backward_pre(self, _module, _grad_output) -> None:
        hook_enter(BACKWARD)

    def _backward_post(self, _module, _inputs, _grad_output) -> None:
        hook_exit(BACKWARD)

    def _step_pre(self, _optimizer, _args, _kwargs) -> None:
        hook_enter(OPTIMIZER)

    def _step_post(self, _optimizer, _args, _kwargs) -> None:
        hook_exit(OPTIMIZER)


def attach_training_phases(model, optimizer) -> PhaseTracker:
    key = (id(model), id(optimizer))
    if key in _REGISTRY:
        return _REGISTRY[key]
    tracker = PhaseTracker(model, optimizer)
    tracker.install()
    _REGISTRY[key] = tracker
    return tracker


def detach_training_phases(model, optimizer) -> None:
    key = (id(model), id(optimizer))
    tracker = _REGISTRY.pop(key, None)
    if tracker is not None:
        tracker.uninstall()


def owns_training_phases(*, model=None, optimizer=None, module=None) -> bool:
    """True when ``attach_training_phases`` owns iteration-level phase spans.

    * **optimizer** — same optimizer instance passed to ``attach_training_phases``.
    * **model** — root model id match.
    * **module** — *module* is the registered root or any of its submodules.
    """
    if model is not None:
        mid = id(model)
        return any(k[0] == mid for k in _REGISTRY)
    if optimizer is not None:
        oid = id(optimizer)
        return any(k[1] == oid for k in _REGISTRY)
    if module is not None:
        mid = id(module)
        for tracker in _REGISTRY.values():
            root = tracker.model
            if mid == id(root):
                return True
            for sub in root.modules():
                if id(sub) == mid:
                    return True
        return False
    return bool(_REGISTRY)


def maybe_auto_attach(optimizer) -> Optional[PhaseTracker]:
    if not _phases_enabled():
        return None
    for (_mid, oid), tracker in _REGISTRY.items():
        if oid == id(optimizer):
            return tracker
    try:
        import probing
        from probing.profiling.torch.module_utils import get_toplevel_module
    except Exception:
        return None
    if not probing.is_enabled():
        return None
    models = get_toplevel_module()
    if not models:
        return None
    tracker = None
    for model in models:
        tracker = attach_training_phases(model, optimizer)
    return tracker


def _phases_enabled() -> bool:
    try:
        import probing

        spec = probing.config.get_str("probing.torch.phases")
        if spec is None or spec == "":
            return True
        return spec.lower() in ("1", "true", "on", "yes")
    except Exception:
        return True

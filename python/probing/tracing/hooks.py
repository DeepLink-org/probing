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
        # Lazily cached ``id()`` of the root and every submodule. The module tree
        # is static during a training run, so build once instead of rescanning
        # ``model.modules()`` on every ``owns_training_phases`` call (that scan is
        # O(modules) per module-hook → O(modules^2) per step on large models).
        self._member_ids: Optional[frozenset[int]] = None

    def contains_module(self, mid: int) -> bool:
        ids = self._member_ids
        if ids is None:
            ids = frozenset(id(sub) for sub in self.model.modules())
            self._member_ids = ids
        return mid in ids

    def install(self) -> None:
        if self._handles:
            return
        m = self.model
        opt = self.optimizer
        # No module backward hooks: PyTorch's shared ``BackwardHook`` machinery
        # (used by both ``register_full_backward_hook`` and its ``_pre`` variant)
        # spams a UserWarning every step when the root model's inputs need no grad,
        # and fires at backward *start* anyway. Instead we arm a one-shot tensor
        # hook on the forward output to mark BACKWARD start, and close BACKWARD at
        # the next forward (grad accumulation) or the optimizer step.
        self._handles = [
            m.register_forward_pre_hook(self._forward_pre),
            m.register_forward_hook(self._forward_post),
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
        self._member_ids = None

    def _forward_pre(self, module, _inputs) -> None:
        if module.training:
            # Close a prior micro-step's backward (grad accumulation) before the
            # next forward opens; no-op on the first forward of an iteration.
            hook_exit(BACKWARD)
            hook_enter(FORWARD)

    def _forward_post(self, module, _inputs, output) -> None:
        if module.training:
            hook_exit(FORWARD)
            self._arm_backward_enter(output)

    def _arm_backward_enter(self, output) -> None:
        """Mark BACKWARD start via tensor hooks on grad-bearing forward outputs.

        Tensor ``register_hook`` avoids PyTorch's module ``BackwardHook`` warning;
        ``_on_output_grad`` opening BACKWARD is idempotent so registering on every
        grad-bearing output is safe (the earliest grad wins).
        """
        import torch

        stack = [output]
        while stack:
            item = stack.pop()
            if isinstance(item, torch.Tensor):
                if item.requires_grad:
                    item.register_hook(self._on_output_grad)
            elif isinstance(item, (tuple, list)):
                stack.extend(item)
            elif isinstance(item, dict):
                stack.extend(item.values())

    def _on_output_grad(self, grad):
        hook_enter(BACKWARD)
        return grad

    def _step_pre(self, _optimizer, _args, _kwargs) -> None:
        hook_exit(BACKWARD)
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
        return any(tracker.contains_module(mid) for tracker in _REGISTRY.values())
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

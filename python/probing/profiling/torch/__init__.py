"""
Torch Profiling

Spec
----
This module implements profiling hooks for PyTorch training loops.

Responsibilities:
1.  Install/Uninstall hooks on PyTorch Modules and Optimizers.
2.  Track forward passes and optimizer steps (backward hooks optional; disabled
    by default because they can alter autograd behaviour).

The training step coordinate is owned by Rust (``probing.tracing.step_snapshot``);
there is no separate Python step counter here.

Public Interfaces:
- `install_hooks`: Attach profiling hooks to a model/optimizer.
- `uninstall_hooks`: Remove attached hooks.
"""

import torch

from ..types import BaseTracer
from .module_utils import module_analysis, module_name

__all__ = ["install_hooks", "uninstall_hooks"]

HOOK_CACHE = {}
EVENT_COUNT = 0
TOTAL_COUNT = 0


def install_hooks(
    m: torch.nn.Module = None,
    opt: torch.optim.Optimizer = None,
    tracer: BaseTracer = None,
    backward: bool = False,
):
    """Attach profiler hooks. ``backward`` is off by default for autograd safety.

    When ``backward`` is enabled on :class:`~probing.profiling.torch_probe.TorchProbe`,
    backward timing uses output-tensor ``register_hook`` in ``post_forward_hook``
    (not module backward hooks, which break with inplace activations).
    """
    if tracer is None:
        return

    global HOOK_CACHE
    if m is not None:
        if id(m) in HOOK_CACHE:
            return
        module_analysis(m)
        h1 = m.register_forward_pre_hook(tracer.pre_forward_hook)
        h2 = m.register_forward_hook(tracer.post_forward_hook)
        HOOK_CACHE[id(m)] = (h1, h2)
        for s in m.children():
            install_hooks(s, tracer=tracer, backward=backward)

    if opt is not None:
        module_name(opt, opt.__class__.__name__)
        h1 = opt.register_step_pre_hook(tracer.pre_step_hook)
        h2 = opt.register_step_post_hook(tracer.post_step_hook)
        HOOK_CACHE[opt] = (h1, h2)


def uninstall_hooks(m=None):
    global HOOK_CACHE
    for k, v in HOOK_CACHE.items():
        if isinstance(v, tuple):
            for h in v:
                h.remove()
    HOOK_CACHE = {}

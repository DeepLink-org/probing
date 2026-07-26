"""Scripted Megatron-style debug loop on the fake device (default: meta)."""

from __future__ import annotations

import logging
from dataclasses import dataclass
from typing import Optional

logger = logging.getLogger(__name__)


@dataclass
class ScriptedLoopResult:
    steps: int
    role: str
    device: str
    last_iteration: int


def run_scripted_loop(
    *,
    steps: int = 4,
    tp: int = 0,
    pp: int = 0,
    dp: int = 0,
    micro_batches: int = 2,
    device: Optional[str] = None,
) -> ScriptedLoopResult:
    """Install fakes (if needed), sync Megatron hooks, run scripted ``train_step``.

    Places a tiny ``nn.Module`` on the fake device for shape-level sanity. Does
    **not** run a real Megatron forward — ``meta`` cannot compute.
    """
    import torch.nn as nn

    import probing
    from probing.ext import megatron as megatron_ext

    from . import install, is_installed, target_device
    from .specs import megatron as megatron_spec

    if not is_installed():
        install(
            device=device,
            specs=(
                "megatron",
                "transformer_engine",
                "apex",
                "flash_attn",
                "triton",
            ),
            force=True,
        )

    megatron_spec.build_megatron_tree(
        tp=tp,
        pp=pp,
        dp=dp,
        iteration=0,
        micro_batches=micro_batches,
        initialized=True,
    )
    megatron_spec.ensure_sys_modules(tp=tp, pp=pp, dp=dp)

    megatron_ext.init_parallel_state()
    megatron_ext.init_training()

    role = probing.current_role() or ""
    dev = target_device()

    model = nn.Linear(8, 4).to("cuda")
    assert next(model.parameters()).device.type == dev, (
        f"expected params on {dev}, got {next(model.parameters()).device}"
    )

    from megatron.training.training import train_step  # type: ignore

    n = max(1, int(steps))
    for i in range(n):
        megatron_spec.set_iteration(i)
        train_step()

    state = megatron_spec.get_state()
    logger.info(
        "scripted loop done: steps=%s role=%s device=%s iteration=%s",
        steps,
        role,
        dev,
        state["iteration"],
    )
    return ScriptedLoopResult(
        steps=int(steps),
        role=role,
        device=dev,
        last_iteration=int(state["iteration"]),
    )

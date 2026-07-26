"""Scripted ``pretrain()`` entry used by fake ``megatron.training``."""

from __future__ import annotations

import logging
import time
from dataclasses import dataclass
from typing import Any, Callable, Optional

logger = logging.getLogger(__name__)


@dataclass
class PretrainResult:
    train_iters: int
    role: str
    device: str
    last_iteration: int
    elapsed_sec: float


def run_pretrain(
    *,
    train_iters: int = 4,
    micro_batches: int = 1,
    model_provider: Optional[Callable[..., Any]] = None,
    forward_step_func: Optional[Callable[..., Any]] = None,
    train_valid_test_datasets_provider: Optional[Callable[..., Any]] = None,
    **_ignored: Any,
) -> PretrainResult:
    """Run a Megatron-shaped pretrain loop without real compute.

    Builds an optional meta-device model via ``model_provider``, advances
    ``iteration`` / probing step coordinates, and invokes the scripted
    ``train_step`` (wrapped by ``probing.ext.megatron`` when enabled).
    """
    import torch.nn as nn

    import probing
    from probing.ext import megatron as megatron_ext

    from . import install, is_installed, target_device
    from .specs import megatron as megatron_spec

    if not is_installed():
        install(
            force=True,
            specs=(
                "megatron",
                "transformer_engine",
                "apex",
                "flash_attn",
                "triton",
            ),
        )

    args = megatron_spec.get_args()
    train_iters = int(getattr(args, "train_iters", train_iters) or train_iters)
    micro_batches = int(
        getattr(args, "global_batch_size", micro_batches)
        // max(1, int(getattr(args, "micro_batch_size", 1) or 1))
        if getattr(args, "global_batch_size", None)
        else micro_batches
    )
    micro_batches = max(1, micro_batches)

    megatron_spec.set_micro_batches(micro_batches)
    megatron_spec.ensure_sys_modules()
    megatron_ext.init_parallel_state()
    megatron_ext.init_training()

    model = None
    if callable(model_provider):
        try:
            model = model_provider(pre_process=True, post_process=True)
        except TypeError:
            # Newer providers may require a builder callable as first arg.
            try:
                from megatron.core.models.gpt import GPTModel

                def _default_builder(args, pre_process, post_process, *a, **k):
                    del args, a, k
                    return GPTModel(pre_process=pre_process, post_process=post_process)

                model = model_provider(
                    _default_builder, pre_process=True, post_process=True
                )
            except Exception as exc:
                logger.debug("model_provider skipped: %s", exc)
                model = None
        if isinstance(model, nn.Module):
            model = model.to("cuda")
        elif isinstance(model, (list, tuple)):
            model = [m.to("cuda") if isinstance(m, nn.Module) else m for m in model]
    else:
        from megatron.core.models.gpt import GPTModel

        model = GPTModel().to("cuda")

    if callable(train_valid_test_datasets_provider):
        try:
            train_valid_test_datasets_provider([16, 0, 0])
        except Exception as exc:
            logger.debug("dataset provider skipped: %s", exc)

    from megatron.training.training import train_step  # type: ignore

    role = probing.current_role() or ""
    device = target_device()
    t0 = time.perf_counter()

    n = max(1, train_iters)
    for i in range(n):
        megatron_spec.set_iteration(i)
        if callable(forward_step_func) and model is not None:
            try:
                data_iter = iter(())
                forward_step_func(
                    data_iter, model[0] if isinstance(model, list) else model
                )
            except Exception as exc:
                # meta / empty batch — expected; train_step still advances coords
                logger.debug("forward_step scripted fallback: %s", exc)
        train_step(model)

    elapsed = time.perf_counter() - t0
    state = megatron_spec.get_state()
    result = PretrainResult(
        train_iters=n,
        role=role,
        device=device,
        last_iteration=int(state["iteration"]),
        elapsed_sec=elapsed,
    )
    try:
        from probing.fakes.journal import record_fake_event
        from probing.fakes.verify import verify_against_probing

        record_fake_event(
            "pretrain",
            "done",
            attrs={
                "train_iters": n,
                "role": role,
                "device": device,
                "elapsed_sec": round(elapsed, 4),
            },
        )
        report = verify_against_probing(require_train_steps=n)
        if not report.ok:
            logger.warning(
                "fake pretrain verify issues: %s",
                [(i.code, i.message, i.expected, i.observed) for i in report.issues],
            )
        else:
            logger.info("fake pretrain verify ok (checked=%s)", report.checked)
    except Exception as exc:
        logger.debug("verify skipped: %s", exc)
    logger.info(
        "fake pretrain done: iters=%s role=%s device=%s elapsed=%.3fs",
        result.train_iters,
        result.role,
        result.device,
        result.elapsed_sec,
    )
    print(
        f"> fake pretrain done: iters={result.train_iters} "
        f"role={result.role} device={result.device} "
        f"elapsed={result.elapsed_sec:.3f}s",
        flush=True,
    )
    return result

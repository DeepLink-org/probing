#!/usr/bin/env python3
"""Megatron-LM-shaped ``pretrain_gpt.py`` entry for macOS / meta-device debugging.

This is **not** NVIDIA's real ``pretrain_gpt.py``. It speaks the same CLI dialect
and import surface under ``probing.fakes`` so you can debug probing role/step /
observability locally without CUDA.

    PROBING=1 python examples/megatron/pretrain_gpt.py \\
      --num-layers 2 --hidden-size 64 --num-attention-heads 4 \\
      --seq-length 32 --micro-batch-size 1 --global-batch-size 2 \\
      --train-iters 4 --mock-data

Or::

    python -m probing.fakes pretrain_gpt --train-iters 4
"""

from __future__ import annotations

import argparse
import os
import sys


def _bootstrap_fakes() -> None:
    os.environ.setdefault("PROBING_FAKES", "1")
    os.environ.setdefault("PROBING_FAKES_FORCE", "1")
    os.environ.setdefault("PROBING_FAKE_DEVICE", "meta")
    os.environ.setdefault("PROBING_MEGATRON", "on")
    os.environ.setdefault("PROBING_MEGATRON_STEP_SYNC", "on")
    os.environ.setdefault("PROBING_NCCL_MOCK", "1")

    from probing.fakes import install

    install(force=True)


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    p = argparse.ArgumentParser(description="Fake Megatron GPT pretrain (meta device)")
    p.add_argument("--num-layers", type=int, default=2)
    p.add_argument("--hidden-size", type=int, default=64)
    p.add_argument("--num-attention-heads", type=int, default=4)
    p.add_argument("--seq-length", type=int, default=32)
    p.add_argument("--micro-batch-size", type=int, default=1)
    p.add_argument("--global-batch-size", type=int, default=2)
    p.add_argument("--train-iters", type=int, default=4)
    p.add_argument("--tensor-model-parallel-size", type=int, default=1)
    p.add_argument("--pipeline-model-parallel-size", type=int, default=1)
    p.add_argument("--vocab-size", type=int, default=1024)
    p.add_argument("--seed", type=int, default=1234)
    p.add_argument("--mock-data", action="store_true", default=True)
    p.add_argument("--device", default=None, help="meta|cpu|mps (default: env / meta)")
    return p.parse_args(argv)


def model_provider(pre_process=True, post_process=True):
    from megatron.core.models.gpt import GPTModel
    from megatron.training import get_args

    args = get_args()
    return GPTModel(
        hidden_size=args.hidden_size,
        vocab_size=args.padded_vocab_size,
        pre_process=pre_process,
        post_process=post_process,
    )


def forward_step(data_iterator, model):
    """Scripted forward: no real tokens; probing still advances via train_step."""
    del data_iterator
    # On meta, even a tiny forward may fail — swallow and return a dummy loss path.
    try:
        out = model()
        return out, lambda loss_mask, output_tensor, model=None: (
            output_tensor.sum() * 0,
            1,
            {"lm loss": output_tensor.sum() * 0},
        )
    except Exception:
        import torch

        z = torch.zeros((), device="cuda")
        return z, lambda *a, **k: (z, 1, {"lm loss": z})


def train_valid_test_datasets_provider(train_val_test_num_samples, vp_stage=None):
    del train_val_test_num_samples, vp_stage
    return None, None, None


def main(argv: list[str] | None = None) -> int:
    args_ns = parse_args(argv)
    if args_ns.device:
        os.environ["PROBING_FAKE_DEVICE"] = args_ns.device

    _bootstrap_fakes()

    from megatron.core.enums import ModelType
    from megatron.training import pretrain, print_rank_0
    from probing.fakes.specs import megatron as megatron_spec

    fake_args = megatron_spec.default_args(
        num_layers=args_ns.num_layers,
        hidden_size=args_ns.hidden_size,
        num_attention_heads=args_ns.num_attention_heads,
        seq_length=args_ns.seq_length,
        max_position_embeddings=args_ns.seq_length,
        micro_batch_size=args_ns.micro_batch_size,
        global_batch_size=args_ns.global_batch_size,
        train_iters=args_ns.train_iters,
        tensor_model_parallel_size=args_ns.tensor_model_parallel_size,
        pipeline_model_parallel_size=args_ns.pipeline_model_parallel_size,
        vocab_size=args_ns.vocab_size,
        padded_vocab_size=args_ns.vocab_size,
        seed=args_ns.seed,
        mock_data=args_ns.mock_data,
    )
    megatron_spec.set_args(fake_args)
    megatron_spec.ensure_sys_modules(args=fake_args)

    print_rank_0("> probing.fakes pretrain_gpt (meta/scripted, not real Megatron-LM)")
    print_rank_0(
        f"> layers={fake_args.num_layers} hidden={fake_args.hidden_size} "
        f"iters={fake_args.train_iters} device={os.environ.get('PROBING_FAKE_DEVICE', 'meta')}"
    )

    result = pretrain(
        train_valid_test_datasets_provider,
        model_provider,
        ModelType.encoder_or_decoder,
        forward_step,
        model_provider=model_provider,
        forward_step_func=forward_step,
        train_valid_test_datasets_provider=train_valid_test_datasets_provider,
    )
    print_rank_0(
        f"> done: iters={result.train_iters} role={result.role} "
        f"device={result.device} elapsed={result.elapsed_sec:.3f}s"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())

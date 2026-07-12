#!/usr/bin/env python3
# Copyright (c) 2025, NVIDIA CORPORATION. All rights reserved.
# Adapted for probing: Megatron-Core distributed training loop + Web UI / spans.
#
# Based on NVIDIA/Megatron-LM ``examples/run_simple_mcore_train_loop.py``.
# Requires: megatron-core, torch, CUDA, torchrun (see ``run_megatron_soak.sh``).
#
# Contract tests (mocked modules): tests/regression/ext/test_megatron_contract.py

from __future__ import annotations

import argparse
import os
import sys
import time
from functools import partial
from pathlib import Path
from typing import Any, Callable, Dict, Iterator, Tuple

import probing
import torch
from torch.optim import Adam
from torch.utils.data import DataLoader

from probing.ext.megatron import (
    maybe_autostart,
    sync_role_from_parallel_state,
    sync_step_from_iteration,
)

try:
    from megatron.core import dist_checkpointing, parallel_state
    from megatron.core.datasets.blended_megatron_dataset_builder import (
        BlendedMegatronDatasetBuilder,
    )
    from megatron.core.datasets.gpt_dataset import GPTDatasetConfig, MockGPTDataset
    from megatron.core.datasets.utils import compile_helpers
    from megatron.core.distributed import DistributedDataParallel, DistributedDataParallelConfig
    from megatron.core.distributed.finalize_model_grads import finalize_model_grads
    from megatron.core.models.gpt.gpt_model import GPTModel
    from megatron.core.models.gpt.gpt_layer_specs import get_gpt_layer_local_spec
    from megatron.core.pipeline_parallel.schedules import get_forward_backward_func
    from megatron.core.tensor_parallel.random import model_parallel_cuda_manual_seed
    from megatron.core.tokenizers import MegatronTokenizer
    from megatron.core.transformer.transformer_config import TransformerConfig
except ImportError as exc:  # pragma: no cover - optional heavy dep
    raise SystemExit(
        "megatron-core is required for this example.\n"
        "  uv pip install megatron-core\n"
        "  docs: https://docs.nvidia.com/megatron-core/developer-guide/latest/get-started/quickstart.html"
    ) from exc

_SEQUENCE_LENGTH = 64


def _wait_for_server_address(*, timeout_sec: float = 5.0) -> str | None:
    deadline = time.monotonic() + timeout_sec
    while time.monotonic() < deadline:
        addr = probing.config.get_str("server.address")
        if addr and str(addr).strip():
            return str(addr).strip().strip("'\"")
        time.sleep(0.1)
    return None


def _print_observability_hints() -> None:
    pid = os.getpid()
    rank = int(os.environ.get("RANK", "0"))
    addr = _wait_for_server_address()
    print(f"[rank {rank}] pid={pid} probing={'on' if probing.is_enabled() else 'off'}")
    if addr:
        print(f"[rank {rank}] Web UI: http://{addr}/")
        if rank == 0:
            print(f"[rank {rank}] Investigate: http://{addr}/investigate")
    elif rank == 0:
        print(f"[rank {rank}] CLI: probing -t {pid} query \"SELECT * FROM python.trace_event LIMIT 8\"")
        print("[rank 0] Tip: set PROBING_PORT=18080 for browser Web UI (see run_megatron_soak.sh)")
    print(flush=True)


def initialize_distributed(
    tensor_model_parallel_size: int,
    pipeline_model_parallel_size: int,
) -> None:
    parallel_state.destroy_model_parallel()

    rank = int(os.environ["RANK"])
    world_size = int(os.environ["WORLD_SIZE"])
    local_rank = int(os.environ["LOCAL_RANK"])

    torch.cuda.set_device(local_rank)
    torch.distributed.init_process_group(
        backend="nccl", rank=rank, world_size=world_size
    )
    parallel_state.initialize_model_parallel(
        tensor_model_parallel_size, pipeline_model_parallel_size
    )


def model_provider() -> GPTModel:
    transformer_config = TransformerConfig(
        num_layers=2,
        hidden_size=12,
        num_attention_heads=4,
        use_cpu_initialization=True,
        pipeline_dtype=torch.float32,
    )
    return GPTModel(
        config=transformer_config,
        transformer_layer_spec=get_gpt_layer_local_spec(),
        vocab_size=100,
        max_sequence_length=_SEQUENCE_LENGTH,
    )


def get_train_data_iterator() -> Iterator:
    if torch.distributed.is_available() and torch.distributed.is_initialized():
        if torch.distributed.get_rank() == 0:
            compile_helpers()
        torch.distributed.barrier()
    else:
        compile_helpers()

    config = GPTDatasetConfig(
        random_seed=0,
        sequence_length=_SEQUENCE_LENGTH,
        reset_position_ids=False,
        reset_attention_mask=False,
        eod_mask_loss=False,
        tokenizer=MegatronTokenizer.from_pretrained(
            metadata_path={"library": "null-text"},
            vocab_size=_SEQUENCE_LENGTH,
        ),
        mid_level_dataset_surplus=0.005,
    )
    datasets = BlendedMegatronDatasetBuilder(
        MockGPTDataset, [1000, None, None], lambda: True, config
    ).build()
    return iter(DataLoader(datasets[0], batch_size=8, shuffle=True))


def forward_step_func(
    data_iterator: Iterator, model: torch.nn.Module
) -> Tuple[torch.Tensor, Callable]:
    def loss_func(
        loss_mask: torch.Tensor, output_tensor: torch.Tensor
    ) -> Tuple[torch.Tensor, Dict[str, torch.Tensor]]:
        losses = output_tensor.float()
        loss_mask = loss_mask.view(-1).float()
        loss = torch.sum(losses.view(-1) * loss_mask) / loss_mask.sum()
        return loss, {"lm loss": loss}

    data = next(data_iterator)
    device = next(model.parameters()).device
    tokens = data["tokens"].to(device)
    attention_mask = data["attention_mask"].to(device)
    position_ids = data["position_ids"].to(device)
    labels = data["labels"].to(device)
    loss_mask = data["loss_mask"].to(device)
    output_tensor = model(tokens, position_ids, attention_mask, labels=labels)
    return output_tensor, partial(loss_func, loss_mask)


def save_distributed_checkpoint(checkpoint_path: str, gpt_model: torch.nn.Module) -> None:
    model = gpt_model.module if hasattr(gpt_model, "module") else gpt_model
    sharded_state_dict = model.sharded_state_dict(prefix="")
    dist_checkpointing.save(
        sharded_state_dict=sharded_state_dict, checkpoint_dir=checkpoint_path
    )


def load_distributed_checkpoint(
    checkpoint_path: str, gpt_model: torch.nn.Module
) -> torch.nn.Module:
    model = gpt_model.module if hasattr(gpt_model, "module") else gpt_model
    sharded_state_dict = model.sharded_state_dict(prefix="")
    checkpoint = dist_checkpointing.load(
        sharded_state_dict=sharded_state_dict, checkpoint_dir=checkpoint_path
    )
    model.load_state_dict(checkpoint)
    return gpt_model


def main() -> None:
    parser = argparse.ArgumentParser(description="Megatron-Core + probing training loop")
    parser.add_argument("--tensor-model-parallel-size", type=int, default=2)
    parser.add_argument("--pipeline-model-parallel-size", type=int, default=1)
    parser.add_argument("--train-iters", type=int, default=0, help="0 = unlimited")
    parser.add_argument("--max-duration-sec", type=int, default=0)
    parser.add_argument("--num-microbatches", type=int, default=1)
    parser.add_argument("--step-sleep-ms", type=int, default=0)
    parser.add_argument("--print-freq", type=int, default=10)
    parser.add_argument("--skip-checkpoint", action="store_true")
    args = parser.parse_args()

    if not torch.cuda.is_available():
        raise SystemExit("CUDA is required for the Megatron-Core training example.")

    world_size = int(os.environ.get("WORLD_SIZE", "1"))
    tp = args.tensor_model_parallel_size
    pp = args.pipeline_model_parallel_size
    if world_size < tp * pp:
        raise SystemExit(
            f"WORLD_SIZE={world_size} must be >= tensor_parallel({tp}) * pipeline_parallel({pp})"
        )

    initialize_distributed(tp, pp)
    maybe_autostart()
    sync_role_from_parallel_state()
    model_parallel_cuda_manual_seed(123)

    rank = torch.distributed.get_rank()
    if rank == 0:
        _print_observability_hints()
        print(f"role={probing.current_role()!r}", flush=True)

    gpt_model = model_provider().to(torch.device("cuda"))
    ddp_config = DistributedDataParallelConfig(
        grad_reduce_in_fp32=False,
        overlap_grad_reduce=False,
        use_distributed_optimizer=False,
    )
    gpt_model = DistributedDataParallel(
        config=gpt_model.config,
        ddp_config=ddp_config,
        module=gpt_model,
    )
    optim = Adam(gpt_model.parameters())
    train_iterator = get_train_data_iterator()
    forward_backward_func = get_forward_backward_func()

    started = time.monotonic()
    iteration = 0
    while True:
        if args.train_iters > 0 and iteration >= args.train_iters:
            break
        if args.max_duration_sec > 0 and (time.monotonic() - started) >= args.max_duration_sec:
            break

        with probing.span("train_iter"):
            optim.zero_grad()
            losses_reduced = forward_backward_func(
                forward_step_func=forward_step_func,
                data_iterator=train_iterator,
                model=gpt_model,
                num_microbatches=args.num_microbatches,
                seq_length=_SEQUENCE_LENGTH,
                micro_batch_size=8,
                decoder_seq_length=_SEQUENCE_LENGTH,
                forward_only=False,
            )
            finalize_model_grads([gpt_model])
            optim.step()
            sync_step_from_iteration(
                iteration + 1,
                micro_batches=args.num_microbatches,
                force=True,
            )
            sync_role_from_parallel_state()

        if rank == 0 and (iteration + 1) % args.print_freq == 0:
            snap = probing.step.snapshot()
            print(
                f"iter={iteration + 1} losses={losses_reduced} "
                f"local_step={snap.local_step} role={probing.current_role()!r}",
                flush=True,
            )

        iteration += 1
        if args.step_sleep_ms > 0:
            time.sleep(args.step_sleep_ms / 1000.0)

    if rank == 0 and not args.skip_checkpoint:
        ckpt_path = os.path.join(os.getcwd(), "ckpt")
        Path(ckpt_path).mkdir(exist_ok=True)
        save_distributed_checkpoint(gpt_model=gpt_model, checkpoint_path=ckpt_path)
        gpt_model = load_distributed_checkpoint(
            gpt_model=gpt_model, checkpoint_path=ckpt_path
        )
        print("checkpoint save/load OK", flush=True)

    if rank == 0:
        print(f"finished after {iteration} iteration(s)", flush=True)
        _print_observability_hints()


if __name__ == "__main__":
    main()

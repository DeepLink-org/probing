#!/usr/bin/env python3
"""Sleep demo for multi-node torchrun + probing hierarchical cluster report.

Run via ``./examples/run_cluster_multinode.sh`` (see script for N nodes × M GPUs).

Probing loads from ``PROBING=2`` (site hook). Rust ctor starts hierarchical
cluster heartbeat when ``WORLD_SIZE > 1`` (default on).
"""

from __future__ import annotations

import os
import time

import torch.distributed as dist


def main() -> None:
    backend = os.environ.get("PROBING_TORCH_BACKEND", "gloo")
    dist.init_process_group(backend=backend)

    rank = dist.get_rank()
    world = dist.get_world_size()
    sleep_sec = float(os.environ.get("SLEEP_SEC", "120"))
    interval = float(os.environ.get("PRINT_INTERVAL_SEC", "30"))

    print(
        f"[rank {rank}/{world}] "
        f"local_rank={os.environ.get('LOCAL_RANK', '?')} "
        f"group_rank={os.environ.get('GROUP_RANK', os.environ.get('NODE_RANK', '?'))} "
        f"sleep {sleep_sec}s",
        flush=True,
    )

    deadline = time.monotonic() + sleep_sec
    while time.monotonic() < deadline:
        remaining = deadline - time.monotonic()
        time.sleep(min(interval, remaining))
        if time.monotonic() < deadline:
            print(f"[rank {rank}] alive", flush=True)

    dist.barrier()
    dist.destroy_process_group()


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""torchrun crash demo — run via ``./examples/run_crash_torchrun.sh``.

Probing loads from ``PROBING=…`` (site hook); crash capture needs no imports here.
Modes: ``record`` (thread exception, process stays up), ``exception`` / ``all`` (fatal).
"""

from __future__ import annotations

import argparse
import os
import threading

import torch.distributed as dist


def _crash(rank: int, *, record_only: bool) -> None:
    def boom() -> None:
        raise RuntimeError(f"demo crash on rank {rank}")

    if record_only:
        t = threading.Thread(target=boom, name=f"crash-r{rank}", daemon=True)
        t.start()
        t.join()
    else:
        boom()


def main() -> None:
    p = argparse.ArgumentParser()
    p.add_argument("--mode", choices=("record", "exception", "all"), default="record")
    p.add_argument("--crash-rank", type=int, default=1)
    p.add_argument("--backend", default=os.environ.get("PROBING_TORCH_BACKEND", "gloo"))
    args = p.parse_args()

    dist.init_process_group(backend=args.backend)
    rank, world = dist.get_rank(), dist.get_world_size()
    print(f"[rank {rank}/{world}] ready", flush=True)

    dist.barrier()
    if args.mode == "all" or rank == args.crash_rank:
        _crash(rank, record_only=args.mode == "record")

    if args.mode == "record":
        dist.barrier()

    dist.destroy_process_group()


if __name__ == "__main__":
    main()

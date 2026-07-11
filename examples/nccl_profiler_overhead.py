#!/usr/bin/env python3
"""End-to-end NCCL collective benchmark — baseline vs probing-nccl-profiler.

Measures AllReduce (or AllGather) throughput under ``torchrun``. Run once per
mode (baseline / profiled) and compare with ``run_nccl_profiler_bench.sh``,
or pass ``--compare`` on rank 0 after two JSON result files exist.

Typical usage (Linux + CUDA + built plugin)::

    make nccl-profiler-lib
    ./examples/run_nccl_profiler_bench.sh

Manual single run (profiled)::

    export NCCL_PROFILER_PLUGIN=$(python -m probing.nccl --plugin-path)
    export NCCL_PROFILE_EVENT_MASK=$(python -m probing.nccl --event-mask)
    export PROBING=2
    export PROBING_NCCL_INFLIGHT_THRESHOLD_SECS=0   # no watchdog noise
    torchrun --nproc_per_node=8 examples/nccl_profiler_overhead.py \\
        --output /tmp/nccl_bench_profiled.json
"""

from __future__ import annotations

import argparse
import json
import os
import statistics
import sys
import time
from dataclasses import asdict, dataclass
from pathlib import Path


@dataclass
class BenchResult:
    profiled: bool
    world_size: int
    local_rank: int
    op: str
    dtype: str
    msg_bytes: int
    warmup_iters: int
    bench_iters: int
    latency_us_mean: float
    latency_us_p50: float
    latency_us_p99: float
    throughput_gbs: float
    total_sec: float
    nccl_profiler_plugin: str | None
    probing_data_dir: str | None

    def overhead_vs(self, baseline: BenchResult) -> dict[str, float]:
        """Return relative slowdown metrics (positive = profiler slower)."""
        return {
            "latency_mean_pct": pct_delta(
                baseline.latency_us_mean, self.latency_us_mean
            ),
            "latency_p50_pct": pct_delta(baseline.latency_us_p50, self.latency_us_p50),
            "latency_p99_pct": pct_delta(baseline.latency_us_p99, self.latency_us_p99),
            "throughput_pct": pct_delta(self.throughput_gbs, baseline.throughput_gbs),
        }


def pct_delta(reference: float, measured: float) -> float:
    if reference == 0:
        return 0.0
    return 100.0 * (measured - reference) / reference


def _percentile(values: list[float], pct: float) -> float:
    if not values:
        return 0.0
    ordered = sorted(values)
    idx = min(len(ordered) - 1, int(round((pct / 100.0) * (len(ordered) - 1))))
    return ordered[idx]


def run_benchmark(args: argparse.Namespace) -> BenchResult:
    import torch
    import torch.distributed as dist

    def dtype_from_name(name: str) -> torch.dtype:
        table = {
            "fp16": torch.float16,
            "bf16": torch.bfloat16,
            "fp32": torch.float32,
        }
        try:
            return table[name]
        except KeyError as exc:
            raise SystemExit(
                f"unsupported dtype {name!r}; choose fp16|bf16|fp32"
            ) from exc

    def element_bytes(dtype: torch.dtype) -> int:
        return torch.tensor([], dtype=dtype).element_size()

    def make_tensor(
        op: str, msg_bytes: int, dtype: torch.dtype, device: torch.device
    ) -> torch.Tensor | list[torch.Tensor]:
        elem = element_bytes(dtype)
        if op == "allreduce":
            count = max(1, msg_bytes // elem)
            return torch.ones(count, dtype=dtype, device=device)
        if op == "allgather":
            count = max(1, msg_bytes // elem)
            return torch.ones(count, dtype=dtype, device=device)
        raise SystemExit(f"unsupported op {op!r}")

    def run_collective(op: str, tensor: torch.Tensor | list[torch.Tensor]) -> None:
        if op == "allreduce":
            dist.all_reduce(tensor)  # type: ignore[arg-type]
        elif op == "allgather":
            world = dist.get_world_size()
            out = [torch.empty_like(tensor) for _ in range(world)]  # type: ignore[arg-type]
            dist.all_gather(out, tensor)  # type: ignore[arg-type]
        else:
            raise RuntimeError(op)

    def sync(device: torch.device) -> None:
        if device.type == "cuda":
            torch.cuda.synchronize(device)

    if not torch.cuda.is_available():
        raise SystemExit("CUDA is required for NCCL overhead benchmark")

    local_rank = int(os.environ.get("LOCAL_RANK", "0"))
    device = torch.device(f"cuda:{local_rank}")
    torch.cuda.set_device(device)

    backend = os.environ.get("PROBING_TORCH_BACKEND", "nccl")
    dist.init_process_group(backend=backend)

    rank = dist.get_rank()
    world = dist.get_world_size()
    dtype = dtype_from_name(args.dtype)
    tensor = make_tensor(args.op, args.msg_bytes, dtype, device)

    for _ in range(args.warmup_iters):
        run_collective(args.op, tensor)
    sync(device)
    dist.barrier()

    latencies_us: list[float] = []
    start_total = time.perf_counter()
    for _ in range(args.bench_iters):
        sync(device)
        t0 = time.perf_counter()
        run_collective(args.op, tensor)
        sync(device)
        latencies_us.append((time.perf_counter() - t0) * 1e6)
    dist.barrier()
    total_sec = time.perf_counter() - start_total

    mean_us = statistics.mean(latencies_us)
    p50_us = _percentile(latencies_us, 50)
    p99_us = _percentile(latencies_us, 99)
    throughput = args.msg_bytes / (mean_us * 1e3) if mean_us > 0 else 0.0

    result = BenchResult(
        profiled=bool(os.environ.get("NCCL_PROFILER_PLUGIN")),
        world_size=world,
        local_rank=local_rank,
        op=args.op,
        dtype=args.dtype,
        msg_bytes=args.msg_bytes,
        warmup_iters=args.warmup_iters,
        bench_iters=args.bench_iters,
        latency_us_mean=mean_us,
        latency_us_p50=p50_us,
        latency_us_p99=p99_us,
        throughput_gbs=throughput,
        total_sec=total_sec,
        nccl_profiler_plugin=os.environ.get("NCCL_PROFILER_PLUGIN"),
        probing_data_dir=os.environ.get("PROBING_DATA_DIR"),
    )

    if rank == 0:
        print(
            f"[bench] profiled={result.profiled} world={world} "
            f"op={args.op} msg={args.msg_bytes}B dtype={args.dtype}\n"
            f"  mean={mean_us:.1f}us p50={p50_us:.1f}us p99={p99_us:.1f}us "
            f"algobw={throughput:.2f} GB/s ({args.bench_iters} iters, {total_sec:.2f}s)",
            flush=True,
        )
        if args.output:
            Path(args.output).write_text(
                json.dumps(asdict(result), indent=2) + "\n", encoding="utf-8"
            )
            print(f"  wrote {args.output}", flush=True)

    dist.barrier()
    dist.destroy_process_group()
    return result


def compare_results(baseline_path: Path, profiled_path: Path) -> None:
    baseline = BenchResult(**json.loads(baseline_path.read_text(encoding="utf-8")))
    profiled = BenchResult(**json.loads(profiled_path.read_text(encoding="utf-8")))
    delta = profiled.overhead_vs(baseline)

    print("\n=== NCCL profiler overhead ===")
    print(f"baseline : {baseline_path}")
    print(f"profiled : {profiled_path}")
    print(
        f"world_size={baseline.world_size} op={baseline.op} msg={baseline.msg_bytes}B"
    )
    print(
        f"latency mean : {baseline.latency_us_mean:.1f}us -> "
        f"{profiled.latency_us_mean:.1f}us  (+{delta['latency_mean_pct']:.2f}%)"
    )
    print(
        f"latency p50  : {baseline.latency_us_p50:.1f}us -> "
        f"{profiled.latency_us_p50:.1f}us  (+{delta['latency_p50_pct']:.2f}%)"
    )
    print(
        f"latency p99  : {baseline.latency_us_p99:.1f}us -> "
        f"{profiled.latency_us_p99:.1f}us  (+{delta['latency_p99_pct']:.2f}%)"
    )
    print(
        f"algobw       : {baseline.throughput_gbs:.2f} -> "
        f"{profiled.throughput_gbs:.2f} GB/s  ({delta['throughput_pct']:+.2f}%)"
    )


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--op", choices=("allreduce", "allgather"), default="allreduce")
    parser.add_argument("--dtype", choices=("fp16", "bf16", "fp32"), default="fp16")
    parser.add_argument(
        "--msg-bytes",
        type=int,
        default=int(os.environ.get("NCCL_BENCH_MSG_BYTES", str(1 << 20))),
        help="per-rank message size in bytes (default 1 MiB)",
    )
    parser.add_argument("--warmup-iters", type=int, default=20)
    parser.add_argument("--bench-iters", type=int, default=200)
    parser.add_argument(
        "--output",
        type=Path,
        default=None,
        help="JSON path written by rank 0",
    )
    parser.add_argument(
        "--compare",
        nargs=2,
        metavar=("BASELINE_JSON", "PROFILED_JSON"),
        type=Path,
        help="compare two result files and exit (no torchrun needed)",
    )
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> None:
    args = parse_args(argv)
    if args.compare:
        compare_results(args.compare[0], args.compare[1])
        return
    run_benchmark(args)


if __name__ == "__main__":
    main(sys.argv[1:])

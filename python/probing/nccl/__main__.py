"""CLI: ``python -m probing.nccl --plugin-path``"""

from __future__ import annotations

import argparse
import sys

from probing.nccl import DEFAULT_EVENT_MASK, plugin_path
from probing.nccl.mock import seed_mock


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        prog="python -m probing.nccl",
        description="NCCL profiler plugin utilities",
    )
    parser.add_argument(
        "--plugin-path",
        action="store_true",
        help="print absolute path to libprobing_nccl_profiler.so (for NCCL_PROFILER_PLUGIN)",
    )
    parser.add_argument(
        "--event-mask",
        action="store_true",
        help=f"print default NCCL_PROFILE_EVENT_MASK ({DEFAULT_EVENT_MASK})",
    )
    parser.add_argument(
        "--seed-mock",
        action="store_true",
        help="write synthetic nccl.proxy_ops / nccl.net_qp rows (macOS dev, no NCCL)",
    )
    parser.add_argument(
        "--ranks",
        type=int,
        default=8,
        help="world size for --seed-mock (default: 8)",
    )
    parser.add_argument(
        "--ops",
        type=int,
        default=5,
        help="collective ops per rank for --seed-mock (default: 5)",
    )
    args = parser.parse_args(argv)

    if args.plugin_path:
        try:
            print(plugin_path())
        except (OSError, FileNotFoundError) as e:
            print(e, file=sys.stderr)
            return 1
        return 0

    if args.event_mask:
        print(DEFAULT_EVENT_MASK)
        return 0

    if args.seed_mock:
        try:
            counts = seed_mock(ranks=args.ranks, ops_per_rank=args.ops)
        except Exception as e:
            print(f"seed-mock failed: {e}", file=sys.stderr)
            return 1
        for table, n in counts.items():
            print(f"{table}: {n} rows", file=sys.stderr)
        print("ok", file=sys.stderr)
        return 0

    parser.print_help(sys.stderr)
    return 2


if __name__ == "__main__":
    raise SystemExit(main())

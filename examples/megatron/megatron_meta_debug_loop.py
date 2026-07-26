#!/usr/bin/env python3
"""macOS / no-CUDA Megatron-shaped debug loop via ``probing.fakes``.

Uses **meta** (default) to stand in for CUDA. This does **not** run a real
Megatron forward — it exercises probing role/step sync and import fakes.

    PROBING=1 python examples/megatron/megatron_meta_debug_loop.py
    PROBING_FAKE_DEVICE=cpu PROBING=1 python examples/megatron/megatron_meta_debug_loop.py
"""

from __future__ import annotations

import os


def main() -> None:
    os.environ.setdefault("PROBING_FAKES", "1")
    os.environ.setdefault("PROBING_FAKES_FORCE", "1")
    os.environ.setdefault("PROBING_FAKE_DEVICE", "meta")
    os.environ.setdefault("PROBING_MEGATRON", "on")
    os.environ.setdefault("PROBING_MEGATRON_STEP_SYNC", "on")
    os.environ.setdefault("PROBING_NCCL_MOCK", "1")

    from probing.fakes import run_scripted_loop

    result = run_scripted_loop(steps=4, tp=1, pp=0, dp=0, micro_batches=2)
    print(
        f"ok: steps={result.steps} role={result.role} "
        f"device={result.device} last_iteration={result.last_iteration}"
    )
    print("tip: probing -t <pid> query \"SELECT * FROM python.trace_event LIMIT 8\"")


if __name__ == "__main__":
    main()

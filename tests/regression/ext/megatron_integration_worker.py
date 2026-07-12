"""Subprocess worker for Megatron-Core integration tests (``torchrun`` nproc=1).

Run via::

    python -m torch.distributed.run --standalone --nproc_per_node=1 \\
        tests/regression/ext/megatron_integration_worker.py

Imports are deferred to ``main()`` so pytest ``--doctest-modules`` does not pull
in ``megatron.core`` at collection time.
"""

from __future__ import annotations

import json
import os
import sys


def main() -> int:
    import torch
    from megatron.core import parallel_state
    from megatron.core.tensor_parallel.random import model_parallel_cuda_manual_seed

    import probing
    from probing.ext.megatron import (
        maybe_autostart,
        sync_role_from_parallel_state,
        sync_step_from_iteration,
    )

    os.environ.setdefault("PROBING", "1")
    os.environ.setdefault("PROBING_MEGATRON", "on")
    os.environ.setdefault("PROBING_MEGATRON_STEP_SYNC", "on")

    if not torch.cuda.is_available():
        print(json.dumps({"error": "cuda_unavailable"}))
        return 2

    local_rank = int(os.environ.get("LOCAL_RANK", "0"))
    torch.cuda.set_device(local_rank)
    torch.distributed.init_process_group(backend="nccl")

    parallel_state.initialize_model_parallel(1, 1)
    model_parallel_cuda_manual_seed(123)

    maybe_autostart()
    role = sync_role_from_parallel_state()
    sync_step_from_iteration(3, micro_batches=2, force=True)

    snap = probing.step.snapshot()
    print(
        json.dumps(
            {
                "role": probing.current_role(),
                "sync_role": role,
                "local_step": int(snap.local_step),
                "micro_step": int(snap.micro_step),
                "micro_batches": int(snap.micro_batches),
            }
        )
    )

    parallel_state.destroy_model_parallel()
    torch.distributed.destroy_process_group()
    return 0


if __name__ == "__main__":
    sys.exit(main())

"""Megatron-Core integration tests (real ``megatron.core``, CUDA required).

Contract coverage lives in ``test_megatron_contract.py``. These tests run in a
subprocess via ``torchrun`` so distributed init does not leak into pytest.
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
from pathlib import Path

import pytest

torch = pytest.importorskip("torch")

pytestmark = [
    pytest.mark.integration,
    pytest.mark.slow,
    pytest.mark.skipif(not torch.cuda.is_available(), reason="requires a CUDA device"),
]

_WORKER = Path(__file__).with_name("megatron_integration_worker.py")
_REPO_ROOT = _WORKER.parents[3]


def _megatron_core_available() -> bool:
    try:
        import megatron.core  # noqa: F401

        return True
    except ImportError:
        return False


@pytest.mark.skipif(
    not _megatron_core_available(), reason="megatron-core not installed"
)
def test_megatron_core_parallel_state_and_step_sync():
    env = {
        **dict(os.environ),
        "PROBING": "1",
        "PROBING_MEGATRON": "on",
        "PROBING_MEGATRON_STEP_SYNC": "on",
        "MASTER_ADDR": "127.0.0.1",
        "MASTER_PORT": "29593",
    }
    cmd = [
        sys.executable,
        "-m",
        "torch.distributed.run",
        "--standalone",
        "--nproc_per_node=1",
        str(_WORKER),
    ]
    proc = subprocess.run(
        cmd,
        env=env,
        capture_output=True,
        text=True,
        timeout=180,
        cwd=_REPO_ROOT,
    )
    assert proc.returncode == 0, proc.stderr or proc.stdout

    lines = [line for line in proc.stdout.splitlines() if line.strip().startswith("{")]
    assert lines, f"no JSON payload in stdout:\n{proc.stdout}\n{proc.stderr}"
    payload = json.loads(lines[-1])

    assert payload.get("sync_role")
    assert "tp=" in payload["role"]
    assert payload["local_step"] == 3
    assert payload["micro_step"] == 6
    assert payload["micro_batches"] == 2

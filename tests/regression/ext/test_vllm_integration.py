"""vLLM integration tests (real ``vllm`` install when present).

Contract coverage lives in ``test_vllm_contract.py``. The worker runs in a
subprocess to avoid mutating global ``probing.step`` in the pytest process.
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
from pathlib import Path

import pytest

pytestmark = [
    pytest.mark.integration,
    pytest.mark.slow,
]

_WORKER = Path(__file__).with_name("vllm_integration_worker.py")
_REPO_ROOT = _WORKER.parents[3]


def _vllm_available() -> bool:
    try:
        import vllm  # noqa: F401

        return True
    except ImportError:
        return False


@pytest.mark.skipif(not _vllm_available(), reason="vllm not installed")
def test_vllm_import_hook_wiring_subprocess():
    env = {
        **dict(os.environ),
        "PROBING": "1",
        "PROBING_VLLM": "on",
        "PROBING_VLLM_STEP_SYNC": "on",
    }
    proc = subprocess.run(
        [sys.executable, str(_WORKER)],
        env=env,
        capture_output=True,
        text=True,
        timeout=120,
        cwd=_REPO_ROOT,
    )
    assert proc.returncode == 0, proc.stderr or proc.stdout

    lines = [line for line in proc.stdout.splitlines() if line.strip().startswith("{")]
    assert lines, f"no JSON payload in stdout:\n{proc.stdout}\n{proc.stderr}"
    payload = json.loads(lines[-1])

    assert payload.get("vllm_version")
    # Engine module may not load until first LLM construction; hook smoke still passes.
    assert "role" in payload


@pytest.mark.skipif(not _vllm_available(), reason="vllm not installed")
def test_vllm_offline_generate_smoke_subprocess():
    """Real ``LLM.generate`` smoke — opt-in (slow, needs aligned vllm/vllm-metal/mlx-lm)."""
    if os.environ.get("PROBING_VLLM_RUN_GENERATE", "").strip().lower() not in (
        "1",
        "true",
        "yes",
        "on",
    ):
        pytest.skip(
            "set PROBING_VLLM_RUN_GENERATE=1 to run real LLM.generate integration"
        )

    model = os.environ.get(
        "PROBING_VLLM_TEST_MODEL",
        "mlx-community/Qwen2.5-0.5B-Instruct-4bit" if sys.platform == "darwin" else "",
    ).strip()
    if not model:
        pytest.skip("set PROBING_VLLM_TEST_MODEL for offline generate integration")

    env = {
        **dict(os.environ),
        "PROBING": "1",
        "PROBING_VLLM": "on",
        "PROBING_VLLM_STEP_SYNC": "on",
        "PROBING_VLLM_TEST_MODEL": model,
    }
    proc = subprocess.run(
        [sys.executable, str(_WORKER)],
        env=env,
        capture_output=True,
        text=True,
        timeout=600,
        cwd=_REPO_ROOT,
    )
    assert proc.returncode == 0, proc.stderr or proc.stdout

    lines = [line for line in proc.stdout.splitlines() if line.strip().startswith("{")]
    assert lines, f"no JSON payload in stdout:\n{proc.stdout}\n{proc.stderr}"
    payload = json.loads(lines[-1])

    assert payload.get("model") == model
    assert int(payload.get("local_step", 0)) >= 0

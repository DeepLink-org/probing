"""Opt-in: real Megatron-LM ``pretrain_gpt.py`` + bottom-layer fakes.

Skipped unless a Megatron-LM checkout is ready (see
``probing.fakes.megatron_lm.resolve_megatron_lm_root``).

Run::

    # default sibling ../Megatron-LM
    PROBING_MEGATRON_REAL_LM=1 PROBING=1 pytest -m integration \\
      tests/regression/ext/test_megatron_real_lm.py -q

    MEGATRON_LM=/path/to/Megatron-LM PROBING_MEGATRON_REAL_LM=1 \\
      pytest -m integration tests/regression/ext/test_megatron_real_lm.py -q
"""

from __future__ import annotations

import os
import subprocess
import sys
from pathlib import Path

import pytest

from probing.fakes.megatron_lm import resolve_megatron_lm_root, smoke_version_allowed

_REPO_ROOT = Path(__file__).resolve().parents[3]
_RUNNER = _REPO_ROOT / "examples" / "megatron" / "run_megatron_lm_pretrain.py"

pytestmark = [
    pytest.mark.integration,
    pytest.mark.slow,
]


def _opt_in() -> bool:
    return os.environ.get("PROBING_MEGATRON_REAL_LM", "").strip().lower() in {
        "1",
        "true",
        "yes",
        "on",
    }


@pytest.fixture(scope="module")
def megatron_checkout():
    if not _opt_in():
        pytest.skip(
            "set PROBING_MEGATRON_REAL_LM=1 to run official Megatron-LM smoke "
            "(MEGATRON_LM or ../Megatron-LM)"
        )
    co = resolve_megatron_lm_root()
    if not co.ready:
        pytest.skip(f"Megatron-LM not ready: {co.reason}")
    allowed, why = smoke_version_allowed(co)
    if not allowed:
        pytest.skip(why)
    return co


def test_real_megatron_lm_one_train_iter(megatron_checkout):
    """One real train step via bottom fakes (cpu + fake process group)."""
    env = {
        **dict(os.environ),
        "PROBING": os.environ.get("PROBING", "1"),
        "MEGATRON_LM": str(megatron_checkout.root),
        "PROBING_CRASH_NO_GRACE": "1",
        "PROBING_CRASH": "0",
    }
    cmd = [
        sys.executable,
        str(_RUNNER),
        "--train-iters",
        "1",
        "--megatron-lm",
        str(megatron_checkout.root),
    ]
    proc = subprocess.run(
        cmd,
        env=env,
        capture_output=True,
        text=True,
        timeout=300,
        cwd=str(_REPO_ROOT),
    )
    out = (proc.stdout or "") + "\n" + (proc.stderr or "")
    assert proc.returncode == 0, out
    assert "Megatron-Core version" in out or "training ..." in out
    assert "iteration" in out.lower() or "lm loss" in out.lower()

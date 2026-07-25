#!/usr/bin/env python3
"""Run real Megatron-LM ``pretrain_gpt.py`` with probing.fakes bottom-layer only.

Default checkout: ``../Megatron-LM`` (sibling of the probing repo). Override with
``MEGATRON_LM=/path`` or ``--megatron-lm /path``.

    PROBING=1 python examples/megatron/run_megatron_lm_pretrain.py --train-iters 2
"""

from __future__ import annotations

import os
import sys
from pathlib import Path


def main() -> int:
    # Ensure full probing init before fakes (avoid -m package circular import).
    import probing  # noqa: F401

    from probing.fakes.megatron_lm import main as megatron_main

    return megatron_main(sys.argv[1:])


if __name__ == "__main__":
    # Allow running from any cwd.
    root = Path(__file__).resolve().parents[1]
    if str(root / "python") not in sys.path:
        sys.path.insert(0, str(root / "python"))
    raise SystemExit(main())

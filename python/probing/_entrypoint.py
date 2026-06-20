"""Shared process entrypoint detection for site hook and package import."""

from __future__ import annotations

import os
import sys


def current_script_name() -> str:
    try:
        return os.path.basename(sys.argv[0])
    except (IndexError, AttributeError):
        return "<unknown>"


def is_probing_cli() -> bool:
    """True when this process is the ``probing`` CLI (must not auto-load the engine)."""
    if current_script_name() == "probing":
        os.environ["PROBING_CLI_MODE"] = "1"
        return True
    try:
        import __main__

        if hasattr(__main__, "__file__") and __main__.__file__:
            main_file = __main__.__file__
            if "probing" in main_file and "cli" in main_file:
                os.environ["PROBING_CLI_MODE"] = "1"
                return True
    except Exception:
        pass
    return False


_LIGHTWEIGHT_MODULES = ("probing.nccl", "probing.skills", "probing.dev_pth")


def is_lightweight_module() -> bool:
    """``python -m probing.nccl|skills|dev_pth`` must not start the engine."""
    helper_suffixes = (
        "probing/nccl/__main__.py",
        "probing\\nccl\\__main__.py",
        "probing/skills/__main__.py",
        "probing\\skills\\__main__.py",
        "probing/dev_pth.py",
        "probing\\dev_pth.py",
    )
    if sys.argv and any(sys.argv[0].endswith(s) for s in helper_suffixes):
        return True

    try:
        idx = sys.argv.index("-m")
        mod = sys.argv[idx + 1]
    except (ValueError, IndexError):
        return False
    return mod in _LIGHTWEIGHT_MODULES

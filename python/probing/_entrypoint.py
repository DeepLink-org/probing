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


def should_activate_probing() -> bool:
    """True when ``PROBING`` / ``PROBING_ORIGINAL`` targets this process."""
    raw = os.environ.get("PROBING_ORIGINAL") or os.environ.get("PROBING", "0")
    token = raw.strip().lower()
    if token in ("0", "", "false", "no", "off"):
        return False
    if token in ("1", "followed", "2", "nested"):
        return True
    if raw.lower().startswith("regex:"):
        import re

        pattern = raw.split(":", 1)[1]
        candidates: list[str] = [c for c in sys.argv if c]
        candidates.append(current_script_name())
        try:
            import __main__

            main_file = getattr(__main__, "__file__", None)
            if main_file:
                candidates.append(main_file)
                candidates.append(os.path.basename(main_file))
        except Exception:
            pass
        return any(re.search(pattern, c) for c in candidates if c)
    return raw == current_script_name()


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


def is_elastic_supervisor() -> bool:
    """True for torchrun / ``python -m torch.distributed.run`` launcher (not a worker rank)."""
    if os.environ.get("LOCAL_RANK") is not None or os.environ.get("RANK") is not None:
        return False
    if os.environ.get("TORCHELASTIC_RUN_ID"):
        return True
    if current_script_name() == "torchrun":
        return True
    if sys.argv[:1] == ["-m"]:
        if len(sys.argv) >= 2 and not sys.argv[1].startswith("-"):
            return False
        return True
    try:
        idx = sys.argv.index("-m")
        mod = sys.argv[idx + 1]
    except (ValueError, IndexError):
        mod = None
    if mod in ("torch.distributed.run", "torch.distributed.launch"):
        return True
    return any(
        (arg or "").endswith("torchrun") or "torch/distributed/run" in (arg or "")
        for arg in sys.argv
    )

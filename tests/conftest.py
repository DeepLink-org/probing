"""Shared pytest fixtures for all Python tests."""

from __future__ import annotations

import faulthandler
import io
import os
import sys
import threading
import traceback
from pathlib import Path

# Capture Rust/Python crash output as early as possible (before probing loads).
os.environ.setdefault("RUST_BACKTRACE", "1")
os.environ.setdefault("PROBING_RUST_BACKTRACE", "1")

# Fallback when tests run without ``make develop`` (no venv .pth files).
# Skip when ``probing._core`` is already installed (wheel / maturin develop) so we
# do not shadow site-packages with the checkout-only ``python/probing`` tree.
_repo_python = Path(__file__).resolve().parents[1] / "python"
if _repo_python.is_dir():
    _repo_python_str = str(_repo_python)
    _prepend_repo = True
    try:
        import importlib.util

        if importlib.util.find_spec("probing._core") is not None:
            _prepend_repo = False
    except (ImportError, ModuleNotFoundError, ValueError):
        pass
    if _prepend_repo and _repo_python_str not in sys.path:
        sys.path.insert(0, _repo_python_str)


def _enable_faulthandler() -> None:
    """Enable faulthandler even when pytest wraps sys.stderr without fileno."""
    try:
        if hasattr(sys.stderr, "fileno"):
            sys.stderr.fileno()
        faulthandler.enable(all_threads=True, file=sys.stderr)
        return
    except (OSError, ValueError, io.UnsupportedOperation):
        pass
    try:
        err = os.fdopen(os.dup(2), "w", buffering=1)
        faulthandler.enable(all_threads=True, file=err)
    except OSError:
        pass


_enable_faulthandler()


def _thread_excepthook(args: threading.ExceptHookArgs) -> None:
    print(
        f"\n=== uncaught exception in thread {args.thread.name!r} "
        f"(ident={args.thread.ident}) ===",
        file=sys.stderr,
    )
    traceback.print_exception(
        args.exc_type,
        args.exc_value,
        args.exc_traceback,
        file=sys.stderr,
    )
    print("=== end thread exception ===\n", file=sys.stderr)


if hasattr(threading, "excepthook"):
    threading.excepthook = _thread_excepthook

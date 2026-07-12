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
# Skip when a complete ``probing`` wheel is installed so we do not shadow
# site-packages with the checkout-only ``python/probing`` tree.
_repo_python = Path(__file__).resolve().parents[1] / "python"


def _installed_probing_is_complete() -> bool:
    try:
        import importlib.util

        if importlib.util.find_spec("probing._core") is None:
            return False
        for mod in ("probing.skills.loader", "probing.ext", "probing.handlers"):
            if importlib.util.find_spec(mod) is None:
                return False
        return True
    except (ImportError, ModuleNotFoundError, ValueError):
        return False


if _repo_python.is_dir():
    _repo_python_str = str(_repo_python)
    _prepend_repo = not _installed_probing_is_complete()
    if (
        os.environ.get("PROBING_WHEEL_TEST") == "1"
        and not _installed_probing_is_complete()
    ):
        raise RuntimeError(
            "installed wheel is incomplete (missing probing.* Python modules); "
            "run: make frontend && make wheel && make install-wheel"
        )
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


def repo_root() -> Path:
    """Repository root (contains ``pyproject.toml``)."""
    return Path(__file__).resolve().parents[1]


def repo_probing_pkg() -> Path:
    return repo_root() / "python" / "probing"


def is_wheel_install() -> bool:
    """True when ``probing`` is imported from site-packages, not the checkout tree."""
    import probing

    return Path(probing.__file__).resolve().parent != repo_probing_pkg().resolve()

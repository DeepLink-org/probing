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


def _missing_probing_modules() -> list[str]:
    try:
        import importlib.util

        required = (
            "probing._core",
            "probing.skills.loader",
            "probing.ext",
            "probing.handlers",
        )
        return [name for name in required if importlib.util.find_spec(name) is None]
    except (ImportError, ModuleNotFoundError, ValueError):
        return ["probing (import system error)"]


def _installed_probing_is_complete() -> bool:
    return not _missing_probing_modules()


if _repo_python.is_dir():
    _repo_python_str = str(_repo_python)
    _prepend_repo = not _installed_probing_is_complete()
    if (
        os.environ.get("PROBING_WHEEL_TEST") == "1"
        and not _installed_probing_is_complete()
    ):
        missing = ", ".join(_missing_probing_modules())
        raise RuntimeError(
            "installed wheel is incomplete "
            f"(missing: {missing}); "
            "run: make frontend && make wheel && make install-wheel "
            "(install with PROBING=0 — do not set PROBING=1 during pip install)"
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

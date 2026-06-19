"""Shared pytest fixtures for the full test suite."""

from __future__ import annotations

import faulthandler
import io
import os
import sys
import threading
import time
import traceback

import pytest

# Capture Rust/Python crash output as early as possible (before probing loads).
os.environ.setdefault("RUST_BACKTRACE", "1")
os.environ.setdefault("PROBING_RUST_BACKTRACE", "1")


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

_COLLECTIVE_CONFIG_KEYS: tuple[str, ...] = (
    "probing.torch.collective.enable",
    "probing.torch.collective.mode",
    "probing.torch.collective.trace_event",
    "probing.torch.collective.verbose",
    "probing.torch.collective.sync",
    "probing.torch.collective.trace_file",
    "probing.torch.collective.resolve_ranks",
)


@pytest.fixture(scope="session", autouse=True)
def _wait_for_probing_engine():
    """Brief pause so the in-process probing server can finish starting."""
    enabled = os.environ.get("PROBING_ORIGINAL") or os.environ.get("PROBING")
    if enabled and str(enabled).lower() not in ("0", "false", "no", ""):
        time.sleep(1.0)
    yield


@pytest.fixture(autouse=True)
def _reset_collective_config(monkeypatch):
    """Reset collective-related config and rank env between tests."""
    import probing

    monkeypatch.delenv("WORLD_SIZE", raising=False)
    monkeypatch.delenv("RANK", raising=False)
    for key in _COLLECTIVE_CONFIG_KEYS:
        try:
            probing.config.remove(key)
        except Exception:
            pass
    yield
    for key in _COLLECTIVE_CONFIG_KEYS:
        try:
            probing.config.remove(key)
        except Exception:
            pass

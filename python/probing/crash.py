"""Python crash hooks — exception capture only; Rust handles spill and reporting."""

from __future__ import annotations

import faulthandler
import os
import sys
import tempfile
import threading
import traceback
from types import TracebackType
from typing import Callable, Optional, Type

from probing._core import crash_enabled, record_crash

_INSTALLED = False
_PREV_EXCEPTHOOK: Optional[Callable] = None
_PREV_THREAD_EXCEPTHOOK: Optional[Callable] = None


def _call_original_excepthook(
    exc_type: Type[BaseException],
    exc_value: BaseException,
    exc_tb: Optional[TracebackType],
) -> None:
    hook = _PREV_EXCEPTHOOK
    if hook is not None and hook is not _probing_excepthook:
        hook(exc_type, exc_value, exc_tb)
    else:
        sys.__excepthook__(exc_type, exc_value, exc_tb)


def _call_original_thread_excepthook(args: threading.ExceptHookArgs) -> None:
    hook = _PREV_THREAD_EXCEPTHOOK
    if hook is not None and hook is not _probing_thread_excepthook:
        hook(args)
    elif hasattr(threading, "__excepthook__"):
        threading.__excepthook__(args)


def _top_frame(
    exc_type: Type[BaseException],
    exc_value: BaseException,
    exc_tb: Optional[TracebackType],
) -> str:
    if exc_tb is not None:
        frames = traceback.extract_tb(exc_tb)
        if frames:
            last = frames[-1]
            return f"{last.filename}:{last.lineno} in {last.name}"
    return f"{getattr(exc_type, '__name__', type(exc_value).__name__)}:<unknown>"


def _capture_thread_stacks() -> str:
    fd, path = tempfile.mkstemp(suffix=".probing-tb")
    try:
        with os.fdopen(fd, "w") as out:
            faulthandler.dump_traceback(file=out, all_threads=True)
        with open(path, encoding="utf-8") as inp:
            return inp.read()
    except Exception:
        return ""
    finally:
        try:
            os.unlink(path)
        except OSError:
            pass


def _dispatch(
    *,
    kind: str,
    exc_type: Type[BaseException],
    exc_value: BaseException,
    exc_tb: Optional[TracebackType],
    native_backtrace: str = "",
    exit_code: int = 1,
    finalize: bool = True,
    crash_thread: str = "",
) -> int:
    exc_name = getattr(exc_type, "__name__", type(exc_value).__name__)
    tb_text = "".join(traceback.format_exception(exc_type, exc_value, exc_tb))
    top = _top_frame(exc_type, exc_value, exc_tb)
    if not crash_thread:
        try:
            crash_thread = threading.current_thread().name or "MainThread"
        except Exception:
            crash_thread = ""
    thread_stacks = _capture_thread_stacks()

    code = record_crash(
        kind,
        exc_name,
        str(exc_value),
        tb_text,
        top,
        native_backtrace,
        finalize,
        thread_stacks,
        crash_thread,
    )

    if not finalize:
        _ensure_hooks()

    return code if finalize else exit_code


def _probing_excepthook(
    exc_type: Type[BaseException],
    exc_value: BaseException,
    exc_tb: Optional[TracebackType],
) -> None:
    if exc_type is KeyboardInterrupt:
        _call_original_excepthook(exc_type, exc_value, exc_tb)
        return
    _call_original_excepthook(exc_type, exc_value, exc_tb)
    code = _dispatch(
        kind="python_exception",
        exc_type=exc_type,
        exc_value=exc_value,
        exc_tb=exc_tb,
        exit_code=1,
        finalize=True,
    )
    sys.exit(code)


def _probing_thread_excepthook(args: threading.ExceptHookArgs) -> None:
    _call_original_thread_excepthook(args)
    thread = args.thread
    _dispatch(
        kind="thread_exception",
        exc_type=args.exc_type,
        exc_value=args.exc_value,
        exc_tb=args.exc_traceback,
        exit_code=1,
        finalize=False,
        crash_thread=thread.name if thread is not None else "",
    )


def install() -> None:
    global _INSTALLED, _PREV_EXCEPTHOOK, _PREV_THREAD_EXCEPTHOOK
    if not crash_enabled():
        return
    if not _INSTALLED:
        _INSTALLED = True
        try:
            faulthandler.enable(all_threads=True, file=sys.stderr)
        except Exception:
            pass
        _PREV_EXCEPTHOOK = sys.excepthook
        if hasattr(threading, "excepthook"):
            _PREV_THREAD_EXCEPTHOOK = threading.excepthook
    _ensure_hooks()


def _ensure_hooks() -> None:
    if sys.excepthook is not _probing_excepthook:
        sys.excepthook = _probing_excepthook
    if (
        hasattr(threading, "excepthook")
        and threading.excepthook is not _probing_thread_excepthook
    ):
        threading.excepthook = _probing_thread_excepthook


__all__ = ["install"]

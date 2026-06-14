"""Run probing._core calls off the main thread when PyArrow/pandas is loaded.

On macOS, calling the Rust extension from the Python main thread after PyArrow
initializes can SIGSEGV (Arrow native runtime conflict). Dispatching through a
short-lived worker thread avoids the crash.
"""

from __future__ import annotations

import sys
import threading
from typing import Callable, TypeVar

T = TypeVar("T")

_MAIN = threading.main_thread()
_HEAVY_MODULES = frozenset({"pyarrow", "pandas"})


def _needs_dispatch() -> bool:
    return threading.current_thread() is _MAIN and any(
        name in sys.modules for name in _HEAVY_MODULES
    )


def call_native(fn: Callable[..., T], /, *args, **kwargs) -> T:
    if not _needs_dispatch():
        return fn(*args, **kwargs)

    out: list[T] = []
    err: list[BaseException] = []

    def worker() -> None:
        try:
            out.append(fn(*args, **kwargs))
        except BaseException as exc:  # noqa: BLE001
            err.append(exc)

    thread = threading.Thread(target=worker, name="probing-native", daemon=True)
    thread.start()
    thread.join()
    if err:
        raise err[0]
    return out[0]

"""Root pytest hook — keep in sync with ``tests/conftest.py`` path logic."""

from __future__ import annotations

import importlib.util
import os
import sys
from pathlib import Path

# Allow ``import probing`` from the checkout during ``make develop`` runs.
# Skip when a complete wheel is installed so we do not shadow site-packages.
_repo_python = Path(__file__).parent / "python"


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
    _prepend_repo = not _installed_probing_is_complete()
    if os.environ.get("PROBING_WHEEL_TEST") == "1" and not _installed_probing_is_complete():
        missing = ", ".join(_missing_probing_modules())
        raise RuntimeError(
            "installed wheel is incomplete "
            f"(missing: {missing}); "
            "run: make frontend && make wheel && make install-wheel "
            "(install with PROBING=0 — do not set PROBING=1 during pip install)"
        )
    if _prepend_repo:
        _repo_python_str = str(_repo_python)
        if _repo_python_str not in sys.path:
            sys.path.insert(0, _repo_python_str)

# The Rust extension (probing._core) must exist — run once: ``make develop``
# (builds _core into python/probing/ and installs the ``probing`` CLI).
# Release/CI wheel path: ``make wheel && make install-wheel``.

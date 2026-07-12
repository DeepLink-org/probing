"""Root pytest hook — keep in sync with ``tests/conftest.py`` path logic."""

from __future__ import annotations

import importlib.util
import sys
from pathlib import Path

# Allow ``import probing`` from the checkout during ``make develop`` runs.
# Skip when ``probing._core`` is already on sys.path (installed wheel / maturin
# develop) so we do not shadow site-packages with the source-only tree.
_repo_python = Path(__file__).parent / "python"
if _repo_python.is_dir():
    _prepend_repo = True
    try:
        if importlib.util.find_spec("probing._core") is not None:
            _prepend_repo = False
    except (ImportError, ModuleNotFoundError, ValueError):
        pass
    if _prepend_repo:
        _repo_python_str = str(_repo_python)
        if _repo_python_str not in sys.path:
            sys.path.insert(0, _repo_python_str)

# The Rust extension (probing._core) must exist — run once: ``make develop``
# (builds _core into python/probing/ and installs the ``probing`` CLI).
# Release/CI wheel path: ``make wheel && make install-wheel``.

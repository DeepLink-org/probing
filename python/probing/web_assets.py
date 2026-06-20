"""Web UI static assets bundled in the wheel or available in editable installs."""

from __future__ import annotations

import os
from pathlib import Path

_ENV = "PROBING_ASSETS_ROOT"
_BUNDLED_DIRNAME = "_web"


def bundled_web_dir() -> Path | None:
    """Wheel / install tree: ``python/probing/_web/`` (bundled by ``make wheel``)."""
    root = Path(__file__).resolve().parent / _BUNDLED_DIRNAME
    if (root / "index.html").is_file():
        return root
    return None


def dev_web_dir() -> Path | None:
    """Editable checkout: repo ``web/dist/``."""
    root = Path(__file__).resolve().parents[2] / "web" / "dist"
    if (root / "index.html").is_file():
        return root
    return None


def resolve_web_assets_root() -> Path | None:
    """Return the directory that contains ``index.html``, if any."""
    override = os.environ.get(_ENV)
    if override:
        root = Path(override)
        if (root / "index.html").is_file():
            return root
        return None
    if bundled := bundled_web_dir():
        return bundled
    return dev_web_dir()


def configure_assets_root() -> Path | None:
    """Set ``PROBING_ASSETS_ROOT`` for ``probing-server`` when UI files are available."""
    if os.environ.get(_ENV):
        root = Path(os.environ[_ENV])
        if (root / "index.html").is_file():
            return root
        return None
    root = resolve_web_assets_root()
    if root is not None:
        os.environ[_ENV] = str(root)
    return root

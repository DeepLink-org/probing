"""Web UI static assets bundled in the wheel or available in editable installs."""

from __future__ import annotations

import os
from pathlib import Path

_ENV = "PROBING_ASSETS_ROOT"
_BUNDLED_DIRNAME = "bundled_web"


def _package_dir() -> Path:
    return Path(__file__).resolve().parent


def _bundled_web_candidates() -> list[Path]:
    """``dx bundle`` emits ``bundled_web/public/``; legacy wheels may be flat."""
    root = _package_dir() / _BUNDLED_DIRNAME
    return [root / "public", root]


def bundled_web_dir() -> Path | None:
    """Wheel / install tree: ``python/probing/bundled_web[/public]``."""
    for candidate in _bundled_web_candidates():
        if (candidate / "index.html").is_file():
            return candidate
    for rel in ("bundled_web/public", "bundled_web"):
        root = _resource_dir(rel, "index.html")
        if root is not None:
            return root
    return None


def dev_web_dir() -> Path | None:
    """Editable checkout: ``web/dist`` symlink → ``bundled_web/public``."""
    if _running_from_installed_wheel():
        return None
    root = _repo_root_from_editable() / "web" / "dist"
    if (root / "index.html").is_file():
        return root
    return bundled_web_dir()


def _running_from_installed_wheel() -> bool:
    maybe_repo = Path(__file__).resolve().parents[2]
    return not (maybe_repo / "pyproject.toml").is_file()


def _repo_root_from_editable() -> Path:
    return Path(__file__).resolve().parents[2]


def _resource_dir(rel: str, marker: str) -> Path | None:
    try:
        from importlib.resources import as_file, files

        bundle = files("probing")
        for part in rel.split("/"):
            bundle = bundle / part
        if not (bundle / marker).is_file():
            return None
        with as_file(bundle) as path:
            return Path(path)
    except (TypeError, ModuleNotFoundError, FileNotFoundError, OSError):
        return None


def _looks_like_built_ui(root: Path) -> bool:
    """True when ``index.html`` is a Dioxus bundle, not the checkout placeholder."""
    index = root / "index.html"
    if not index.is_file():
        return False
    try:
        body = index.read_text(encoding="utf-8", errors="ignore")
    except OSError:
        return False
    return "web-dxh" in body or '<div id="main">' in body


def resolve_web_assets_root() -> Path | None:
    """Return the directory that contains ``index.html``, if any."""
    override = os.environ.get(_ENV)
    if override:
        root = Path(override)
        if (root / "index.html").is_file():
            return root
        return None

    for getter in (dev_web_dir, bundled_web_dir):
        root = getter()
        if root and _looks_like_built_ui(root):
            return root

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

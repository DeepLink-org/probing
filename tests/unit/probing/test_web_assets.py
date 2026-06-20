"""Tests for wheel / editable web UI asset resolution."""

from __future__ import annotations

import os
from pathlib import Path

import pytest

from probing import web_assets

from tests.conftest import is_wheel_install, repo_root


def test_bundled_web_dir_missing_without_sync():
    root = web_assets.bundled_web_dir()
    checkout_bundled = repo_root() / "python" / "probing" / "bundled_web" / "index.html"
    if is_wheel_install():
        assert root is not None, "installed wheel is missing probing/bundled_web"
        assert (root / "index.html").is_file()
        return
    if root is None:
        assert not checkout_bundled.is_file()
    else:
        assert (root / "index.html").is_file()


def test_dev_web_dir_when_frontend_built():
    root = web_assets.dev_web_dir()
    repo_dist = repo_root() / "web" / "dist" / "index.html"
    if is_wheel_install():
        pytest.skip("dev_web_dir applies to editable checkout layout only")
    if repo_dist.is_file():
        assert root is not None
        assert (root / "index.html").is_file()
    else:
        assert root is None


def test_configure_assets_root_prefers_bundled(monkeypatch, tmp_path: Path):
    bundled = tmp_path / "_web"
    bundled.mkdir()
    (bundled / "index.html").write_text("<html>bundled</html>", encoding="utf-8")

    dev = tmp_path / "web" / "dist"
    dev.mkdir(parents=True)
    (dev / "index.html").write_text("<html>dev</html>", encoding="utf-8")

    monkeypatch.setattr(web_assets, "bundled_web_dir", lambda: bundled)
    monkeypatch.setattr(web_assets, "dev_web_dir", lambda: dev)
    monkeypatch.delenv(web_assets._ENV, raising=False)

    assert web_assets.configure_assets_root() == bundled
    assert os.environ[web_assets._ENV] == str(bundled)


def test_configure_assets_root_respects_override(monkeypatch, tmp_path: Path):
    override = tmp_path / "custom"
    override.mkdir()
    (override / "index.html").write_text("<html>custom</html>", encoding="utf-8")
    monkeypatch.setenv(web_assets._ENV, str(override))

    assert web_assets.configure_assets_root() == override

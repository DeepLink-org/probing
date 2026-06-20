"""Tests for wheel / editable web UI asset resolution."""

from __future__ import annotations

import os
from pathlib import Path

from probing import web_assets


def test_bundled_web_dir_missing_without_sync():
    root = web_assets.bundled_web_dir()
    repo_root = Path(__file__).resolve().parents[3]
    bundled_index = repo_root / "python" / "probing" / "_web" / "index.html"
    if root is None:
        assert not bundled_index.is_file()
    else:
        assert (root / "index.html").is_file()


def test_dev_web_dir_when_frontend_built():
    root = web_assets.dev_web_dir()
    repo_dist = Path(__file__).resolve().parents[3] / "web" / "dist" / "index.html"
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

"""Tests for wheel / editable web UI asset resolution."""

from __future__ import annotations

import os
import zipfile
from pathlib import Path

import pytest

from probing import web_assets
from scripts.verify_web_assets import verify_web_bundle
from scripts.verify_wheel_contents import REQUIRED_PATHS, verify_wheel

from tests.conftest import is_wheel_install, repo_root


def test_bundled_web_dir_missing_without_sync():
    root = web_assets.bundled_web_dir()
    checkout_bundled = (
        repo_root() / "python" / "probing" / "bundled_web" / "public" / "index.html"
    )
    legacy_bundled = repo_root() / "python" / "probing" / "bundled_web" / "index.html"
    if is_wheel_install():
        assert root is not None, "installed wheel is missing probing/bundled_web"
        assert (root / "index.html").is_file()
        return
    if root is None:
        assert not checkout_bundled.is_file() and not legacy_bundled.is_file()
    else:
        assert (root / "index.html").is_file()


def test_dev_web_dir_when_frontend_built():
    root = web_assets.dev_web_dir()
    built = repo_root() / "python" / "probing" / "bundled_web" / "public" / "index.html"
    if is_wheel_install():
        pytest.skip("dev_web_dir applies to editable checkout layout only")
    if built.is_file():
        assert root is not None
        assert (root / "index.html").is_file()
        assert root.resolve() == built.parent.resolve()
    else:
        assert root is None


def test_configure_assets_root_prefers_dev_in_editable(monkeypatch, tmp_path: Path):
    bundled = tmp_path / "_web"
    bundled.mkdir()
    (bundled / "index.html").write_text("<html>bundled</html>", encoding="utf-8")

    dev = tmp_path / "web" / "dist"
    dev.mkdir(parents=True)
    (dev / "index.html").write_text(
        '<html><div id="main"></div><script src="/assets/web-dxhabc.js"></script></html>',
        encoding="utf-8",
    )

    monkeypatch.setattr(web_assets, "bundled_web_dir", lambda: bundled)
    monkeypatch.setattr(web_assets, "dev_web_dir", lambda: dev)
    monkeypatch.setattr(web_assets, "_running_from_installed_wheel", lambda: False)
    monkeypatch.delenv(web_assets._ENV, raising=False)

    assert web_assets.configure_assets_root() == dev
    assert os.environ[web_assets._ENV] == str(dev)


def test_configure_assets_root_prefers_bundled_on_wheel(monkeypatch, tmp_path: Path):
    bundled = tmp_path / "_web"
    bundled.mkdir()
    (bundled / "index.html").write_text(
        '<html><div id="main"></div><script src="/assets/web-dxhabc.js"></script></html>',
        encoding="utf-8",
    )

    dev = tmp_path / "web" / "dist"
    dev.mkdir(parents=True)
    (dev / "index.html").write_text("<html>dev</html>", encoding="utf-8")

    monkeypatch.setattr(web_assets, "bundled_web_dir", lambda: bundled)
    monkeypatch.setattr(web_assets, "dev_web_dir", lambda: dev)
    monkeypatch.setattr(web_assets, "_running_from_installed_wheel", lambda: True)
    monkeypatch.delenv(web_assets._ENV, raising=False)

    assert web_assets.configure_assets_root() == bundled
    assert os.environ[web_assets._ENV] == str(bundled)


def test_configure_assets_root_respects_override(monkeypatch, tmp_path: Path):
    override = tmp_path / "custom"
    override.mkdir()
    (override / "index.html").write_text("<html>custom</html>", encoding="utf-8")
    monkeypatch.setenv(web_assets._ENV, str(override))

    assert web_assets.configure_assets_root() == override


def _write_valid_web_bundle(root: Path) -> None:
    assets = root / "assets"
    assets.mkdir(parents=True)
    (root / "index.html").write_text(
        '<link href="/assets/tailwind.css">'
        '<script type="module" src="/./assets/web-dxhabc123.js"></script>',
        encoding="utf-8",
    )
    (assets / "tailwind.css").write_text("", encoding="utf-8")
    (assets / "web-dxhabc123.js").write_text(
        'const wasm = "web_bg-dxhdef456.wasm";',
        encoding="utf-8",
    )
    (assets / "web_bg-dxhdef456.wasm").write_bytes(b"\0asm")


def test_verify_web_bundle_accepts_complete_reference_graph(tmp_path: Path):
    _write_valid_web_bundle(tmp_path)
    assert verify_web_bundle(tmp_path) == []


def test_verify_web_bundle_rejects_missing_entry_script(tmp_path: Path):
    _write_valid_web_bundle(tmp_path)
    (tmp_path / "assets" / "web-dxhabc123.js").unlink()
    errors = verify_web_bundle(tmp_path)
    assert any(
        "missing asset" in error and "web-dxhabc123.js" in error for error in errors
    )


def test_verify_web_bundle_rejects_missing_wasm(tmp_path: Path):
    _write_valid_web_bundle(tmp_path)
    (tmp_path / "assets" / "web_bg-dxhdef456.wasm").unlink()
    errors = verify_web_bundle(tmp_path)
    assert any("missing WASM module" in error for error in errors)


def test_verify_wheel_rejects_broken_web_reference(tmp_path: Path):
    wheel = tmp_path / "probing-test.whl"
    with zipfile.ZipFile(wheel, "w") as archive:
        for path in REQUIRED_PATHS:
            content = (
                '<script type="module" src="/assets/web-dxhdeadbeef.js"></script>'
                if path.endswith("bundled_web/public/index.html")
                else ""
            )
            archive.writestr(path, content)
    errors = verify_wheel(wheel)
    assert any("invalid web bundle" in error for error in errors)

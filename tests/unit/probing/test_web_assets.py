"""Tests for Web bundle integrity and native-wheel embedding checks."""

from __future__ import annotations

import zipfile
from pathlib import Path

from scripts.verify_web_assets import verify_web_bundle
from scripts.verify_wheel_contents import (
    EMBEDDED_WEB_MARKER,
    REQUIRED_PATHS,
    verify_wheel,
)


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


def test_verify_wheel_rejects_native_extension_without_embedded_web(tmp_path: Path):
    wheel = tmp_path / "probing-test.whl"
    with zipfile.ZipFile(wheel, "w") as archive:
        for path in REQUIRED_PATHS:
            archive.writestr(path, "")
        archive.writestr("probing/_core.test.so", b"native-without-assets")
    errors = verify_wheel(wheel)
    assert "native extension does not contain embedded Web assets" in errors


def test_verify_wheel_accepts_embedded_web_marker(tmp_path: Path):
    wheel = tmp_path / "probing-test.whl"
    with zipfile.ZipFile(wheel, "w") as archive:
        for path in REQUIRED_PATHS:
            archive.writestr(path, "")
        archive.writestr("probing/_core.test.so", b"native" + EMBEDDED_WEB_MARKER)
    assert verify_wheel(wheel) == []

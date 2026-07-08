"""Packaging checks: bundled_skills SSOT + skills/ symlink alias."""

from __future__ import annotations

import os
import zipfile
from pathlib import Path

import pytest

REPO_ROOT = Path(__file__).resolve().parents[3]


def test_skills_symlink_points_at_bundled_skills():
    link = REPO_ROOT / "skills"
    pkg = REPO_ROOT / "python" / "probing" / "bundled_skills"
    assert link.is_symlink()
    assert (link / "catalog.yaml").is_file()
    assert link.resolve() == pkg.resolve()


def test_bundled_skills_dir_uses_package_tree():
    from probing.extensions import bundled_skills_dir

    bundled = bundled_skills_dir()
    assert bundled is not None
    expected = REPO_ROOT / "python" / "probing" / "bundled_skills"
    assert bundled.resolve() == expected.resolve()


@pytest.mark.skipif(
    os.environ.get("PROBING_VERIFY_WHEEL") != "1",
    reason="set PROBING_VERIFY_WHEEL=1 after make wheel to inspect archive",
)
def test_built_wheel_includes_bundled_skills():
    wheels = sorted(
        (REPO_ROOT / "dist").glob("probing-*.whl"), key=lambda p: p.stat().st_mtime
    )
    assert wheels, "run: make wheel"
    whl = wheels[-1]

    with zipfile.ZipFile(whl) as zf:
        entries = [n for n in zf.namelist() if n.startswith("probing/bundled_skills/")]
        assert entries, f"{whl.name} missing probing/bundled_skills/"
        catalogs = [n for n in entries if n.endswith("catalog.yaml")]
        assert len(catalogs) == 1
        assert b"skills:" in zf.read(catalogs[0])

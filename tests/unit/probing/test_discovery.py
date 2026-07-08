"""Tests for ``probing.extensions`` (skills + magics entry points)."""

from __future__ import annotations

import json
from pathlib import Path


def test_skill_roots_json_is_valid():
    from probing.extensions import skill_roots_json

    payload = json.loads(skill_roots_json())
    assert isinstance(payload, list)
    for item in payload:
        assert "path" in item
        assert "label" in item
        assert Path(item["path"]).is_dir()


def test_skill_roots_include_bundled_or_repo():
    from probing.extensions import skill_roots

    paths = [str(r.path) for r in skill_roots()]
    assert any("bundled_skills" in p or p.endswith("/skills") for p in paths)


def test_bundled_skills_resolves_to_package_tree():
    from probing.extensions import bundled_skills_dir

    bundled = bundled_skills_dir()
    assert bundled is not None
    assert (bundled / "catalog.yaml").is_file()
    expected = (
        Path(__file__).resolve().parents[3] / "python" / "probing" / "bundled_skills"
    )
    assert bundled.resolve() == expected.resolve()


def test_vendor_naming_helpers():
    from probing.extensions import (
        vendor_id_from_dist_name,
        vendor_import_name,
        vendor_package_name,
    )

    assert vendor_package_name("nvidia") == "probing-nvidia"
    assert vendor_package_name("huawei") == "probing-huawei"
    assert vendor_import_name("nvidia") == "probing_nvidia"
    assert vendor_id_from_dist_name("probing-nvidia") == "nvidia"
    assert vendor_id_from_dist_name("probing") is None
    assert vendor_id_from_dist_name("nvidia-probing") is None


def test_list_vendor_extensions_returns_list():
    from probing.extensions import list_vendor_extensions

    rows = list_vendor_extensions()
    assert isinstance(rows, list)
    for row in rows:
        assert row["package"].startswith("probing-")
        assert row["vendor"]


def test_entry_point_skill_root_labels():
    from probing.extensions import load_skill_roots_from_entry_points

    for _path, label in load_skill_roots_from_entry_points():
        assert ":" in label


def test_load_magics_no_crash():
    from probing.extensions import load_magics
    from probing.repl import get_registered_magics

    before = set(get_registered_magics())
    registry = get_registered_magics()
    loaded = load_magics(registry)
    assert isinstance(loaded, list)
    assert before.issubset(set(registry))


def test_discovery_shim_reexports():
    from probing import discovery as shim
    from probing.extensions import skill_roots as roots

    assert shim.skill_roots is roots

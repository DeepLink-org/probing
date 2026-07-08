"""Backward-compatible re-exports — prefer ``probing.extensions``."""

from __future__ import annotations

from probing.extensions import (
    ENTRY_GROUP_MAGICS,
    ENTRY_GROUP_SKILLS,
    SkillRoot,
    bundled_skills_dir,
    find_repo_root,
    list_vendor_extensions,
    load_magics,
    load_magics_from_entry_points,
    load_skill_roots_from_entry_points,
    repo_skills_dir,
    skill_root_bundled,
    skill_roots,
    skill_roots_json,
    vendor_extensions_json,
    vendor_id_from_dist_name,
    vendor_import_name,
    vendor_package_name,
)
from probing.extensions.__main__ import main as _main

entry_point_skill_roots = load_skill_roots_from_entry_points
load_entry_point_magics = load_magics_from_entry_points
package_skill_roots = load_skill_roots_from_entry_points

if __name__ == "__main__":
    raise SystemExit(_main())

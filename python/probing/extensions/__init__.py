"""Extension discovery — single boundary for skills, magics, and vendor packages.

Third-party ``probing-<vendor>`` wheels register via setuptools entry points:

- ``probing.skills`` → ``skill_root()`` returning a ``catalog.yaml`` directory
- ``probing.magics`` → ``Magics`` subclass for the REPL

All consumers (skill loader, REPL, HTTP handlers, Rust CLI subprocess) should
import from this package only.
"""

from __future__ import annotations

from typing import Dict, List, Type

from probing.extensions.entrypoints import (
    ENTRY_GROUP_MAGICS,
    ENTRY_GROUP_SKILLS,
    load_magics_from_entry_points,
    load_skill_roots_from_entry_points,
)
from probing.extensions.skills import (
    bundled_skills_dir,
    default_install_source,
    find_repo_root,
    repo_skills_dir,
    resolve_skill_dir,
    skill_root_bundled,
    skill_roots,
    skill_roots_json,
)
from probing.extensions.types import SkillRoot
from probing.extensions.vendor import (
    list_vendor_extensions,
    vendor_extensions_json,
    vendor_id_from_dist_name,
    vendor_import_name,
    vendor_package_name,
)


def load_magics(registry: Dict[str, Type]) -> List[str]:
    """Register ``probing.magics`` entry points into *registry*."""
    return load_magics_from_entry_points(registry)


__all__ = [
    "ENTRY_GROUP_MAGICS",
    "ENTRY_GROUP_SKILLS",
    "SkillRoot",
    "bundled_skills_dir",
    "default_install_source",
    "find_repo_root",
    "list_vendor_extensions",
    "load_magics",
    "load_magics_from_entry_points",
    "load_skill_roots_from_entry_points",
    "repo_skills_dir",
    "resolve_skill_dir",
    "skill_root_bundled",
    "skill_roots",
    "skill_roots_json",
    "vendor_extensions_json",
    "vendor_id_from_dist_name",
    "vendor_import_name",
    "vendor_package_name",
]

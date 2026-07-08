"""``probing-<vendor>`` package naming and inventory."""

from __future__ import annotations

import json
import re
from typing import Any, Dict, Iterable, List, Optional

from probing.extensions.entrypoints import (
    ENTRY_GROUP_MAGICS,
    ENTRY_GROUP_SKILLS,
    entry_point_names_for_dist,
    load_skill_roots_from_entry_points,
)

VENDOR_PACKAGE_PREFIX = "probing-"
_VENDOR_DIST_RE = re.compile(r"^probing-[a-z0-9][a-z0-9-]*$")


def vendor_package_name(vendor: str) -> str:
    slug = vendor.strip().lower().replace("_", "-")
    if not slug or slug == "probing":
        raise ValueError(f"invalid vendor id: {vendor!r}")
    return f"{VENDOR_PACKAGE_PREFIX}{slug}"


def vendor_import_name(vendor: str) -> str:
    slug = vendor.strip().lower().replace("-", "_")
    if not slug or slug == "probing":
        raise ValueError(f"invalid vendor id: {vendor!r}")
    return f"probing_{slug}"


def vendor_id_from_dist_name(dist_name: str) -> Optional[str]:
    name = dist_name.strip().lower()
    if not _VENDOR_DIST_RE.match(name):
        return None
    vendor = name[len(VENDOR_PACKAGE_PREFIX) :]
    return vendor or None


def _installed_distributions() -> Iterable[Any]:
    try:
        from importlib.metadata import distributions

        return distributions()
    except ImportError:
        return []


def list_vendor_extensions() -> List[Dict[str, Any]]:
    """Installed ``probing-<vendor>`` packages and their entry-point contributions."""
    seen: set[str] = set()
    out: List[Dict[str, Any]] = []
    for dist in _installed_distributions():
        dist_name = (dist.metadata.get("Name") or dist.name).strip()
        vendor = vendor_id_from_dist_name(dist_name)
        if vendor is None or vendor in seen:
            continue
        seen.add(vendor)
        version = dist.metadata.get("Version") or getattr(dist, "version", "")
        skill_eps = entry_point_names_for_dist(dist_name, ENTRY_GROUP_SKILLS)
        magic_eps = entry_point_names_for_dist(dist_name, ENTRY_GROUP_MAGICS)
        skill_root_paths: List[str] = []
        for path, label in load_skill_roots_from_entry_points():
            if label.startswith(f"{dist_name}:"):
                skill_root_paths.append(str(path))
        out.append(
            {
                "vendor": vendor,
                "package": dist_name,
                "version": version,
                "import_name": vendor_import_name(vendor),
                "skills_entry_points": skill_eps,
                "magics_entry_points": magic_eps,
                "skill_roots": sorted(set(skill_root_paths)),
            }
        )
    out.sort(key=lambda row: row["vendor"])
    return out


def vendor_extensions_json() -> str:
    return json.dumps(list_vendor_extensions())

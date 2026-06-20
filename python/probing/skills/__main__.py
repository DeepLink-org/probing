"""CLI: python -m probing.skills validate|install|update"""

from __future__ import annotations

import sys

from probing.skills.loader import load_catalog, validate_all
from probing.skills.paths import repo_skills_dir, skill_roots


def _roots_display() -> str:
    roots = skill_roots()
    if not roots:
        return "(none)"
    return ", ".join(f"{r.label}:{r.path}" for r in roots)


def main() -> int:
    if len(sys.argv) > 1 and sys.argv[1] in ("install", "update"):
        from probing.skills.install import main as install_main

        return install_main([sys.argv[1], *sys.argv[2:]])

    roots = skill_roots()
    if not roots:
        print(
            "No skills directory found (repo skills/, bundled, or PROBING_SKILLS_DIR)",
            file=sys.stderr,
        )
        return 1
    catalog = load_catalog()
    print(f"Catalog: {len(catalog.skills)} skills")
    print(f"Roots: {_roots_display()}")
    if repo := repo_skills_dir():
        print(f"Repo skills: {repo}")
    warnings = validate_all()
    if warnings:
        for w in warnings:
            print(f"WARN: {w}", file=sys.stderr)
        return 1
    print("OK — all skills valid")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

"""CLI: python -m probing.playbooks validate"""

from __future__ import annotations

import sys

from probing.playbooks.loader import load_catalog, playbooks_root, validate_all


def main() -> int:
    root = playbooks_root()
    if not root.is_dir():
        print(f"playbooks directory not found: {root}", file=sys.stderr)
        return 1
    catalog = load_catalog(root)
    print(f"Catalog: {len(catalog.playbooks)} playbooks under {root}")
    warnings = validate_all(root)
    if warnings:
        print("Warnings:")
        for w in warnings:
            print(f"  - {w}")
        return 1
    print("OK — all playbooks valid")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

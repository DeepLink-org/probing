"""CLI: ``python -m probing.extensions {skill-roots|extensions}``."""

from __future__ import annotations

import sys
from typing import Optional, Sequence

from probing.extensions.skills import skill_roots_json
from probing.extensions.vendor import vendor_extensions_json


def main(argv: Optional[Sequence[str]] = None) -> int:
    args = list(argv or sys.argv[1:])
    if not args or args[0] in ("-h", "--help"):
        print(
            "Usage: python -m probing.extensions {skill-roots|extensions}",
            file=sys.stderr,
        )
        return 2 if args else 0
    if args[0] == "skill-roots":
        print(skill_roots_json())
        return 0
    if args[0] == "extensions":
        print(vendor_extensions_json())
        return 0
    print(f"Unknown command: {args[0]}", file=sys.stderr)
    return 2


if __name__ == "__main__":
    raise SystemExit(main())

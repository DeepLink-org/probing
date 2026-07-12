#!/usr/bin/env python3
"""Append a PEP 440 local version label for nightly wheel builds."""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
VERSION_RE = re.compile(r'^(version = ")([^"+]+)(")', re.MULTILINE)


def patch(path: Path, local: str) -> None:
    text = path.read_text(encoding="utf-8")
    new, count = VERSION_RE.subn(rf"\g<1>\g<2>+{local}\g<3>", text, count=1)
    if count != 1:
        raise SystemExit(f"failed to patch version in {path}")
    path.write_text(new, encoding="utf-8")


def main() -> None:
    if len(sys.argv) != 2 or not sys.argv[1].strip():
        raise SystemExit("usage: set_nightly_local_version.py <local-label>")
    local = sys.argv[1].strip()
    patch(ROOT / "pyproject.toml", local)
    patch(ROOT / "Cargo.toml", local)
    print(f"patched nightly local version +{local}")


if __name__ == "__main__":
    main()

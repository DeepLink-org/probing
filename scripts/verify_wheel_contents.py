#!/usr/bin/env python3
"""Verify a probing wheel contains bundled assets and the Python package tree."""

from __future__ import annotations

import argparse
import sys
import zipfile
from pathlib import Path

try:
    from verify_web_assets import verify_web_files
except ModuleNotFoundError:  # Imported as `scripts.verify_wheel_contents` in tests.
    from scripts.verify_web_assets import verify_web_files

# Paths that must exist in every release wheel (wheel archive member names).
REQUIRED_PATHS = (
    "probing/__init__.py",
    "probing/ext/__init__.py",
    "probing/skills/loader.py",
    "probing/handlers/router.py",
    "probing/profiling/torch_probe.py",
    "probing/bundled_skills/catalog.yaml",
    "probing/bundled_web/public/index.html",
)


def _pick_wheel(path: Path | None) -> Path:
    if path is not None:
        if not path.is_file():
            raise SystemExit(f"error: wheel not found: {path}")
        return path
    dist = Path("dist")
    wheels = sorted(dist.glob("probing-*.whl"))
    if not wheels:
        raise SystemExit("error: no dist/probing-*.whl (run: make wheel)")
    return wheels[0]


def verify_wheel(wheel: Path) -> list[str]:
    missing: list[str] = []
    with zipfile.ZipFile(wheel) as zf:
        names = set(zf.namelist())
        for member in REQUIRED_PATHS:
            if member not in names:
                # Legacy dx layout (no public/ subdir).
                if member == "probing/bundled_web/public/index.html":
                    if "probing/bundled_web/index.html" in names:
                        continue
                missing.append(member)
        web_root = (
            "probing/bundled_web/public/"
            if "probing/bundled_web/public/index.html" in names
            else "probing/bundled_web/"
        )
        index_member = f"{web_root}index.html"
        if index_member in names:
            errors = verify_web_files(
                zf.read(index_member).decode("utf-8"),
                lambda path: f"{web_root}{path}" in names,
                lambda path: zf.read(f"{web_root}{path}").decode("utf-8"),
            )
            missing.extend(f"invalid web bundle: {error}" for error in errors)
    return missing


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "wheel",
        nargs="?",
        type=Path,
        help="wheel path (default: first dist/probing-*.whl)",
    )
    args = parser.parse_args(argv)
    wheel = _pick_wheel(args.wheel)
    missing = verify_wheel(wheel)
    if missing:
        print(f"error: {wheel} is missing required members:", file=sys.stderr)
        for path in missing:
            print(f"  - {path}", file=sys.stderr)
        print(
            "hint: run 'make frontend && make wheel' before install-wheel",
            file=sys.stderr,
        )
        return 1
    print(f"ok: {wheel.name} ({len(REQUIRED_PATHS)} required paths)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

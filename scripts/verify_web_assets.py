#!/usr/bin/env python3
"""Verify that a bundled Dioxus index references files present in the bundle."""

from __future__ import annotations

import argparse
import re
import sys
from collections.abc import Callable
from pathlib import Path, PurePosixPath
from urllib.parse import unquote, urlsplit

_HTML_ASSET_RE = re.compile(r"""(?:src|href)=["']([^"']+)["']""", re.IGNORECASE)
_ENTRY_JS_RE = re.compile(r"(?:^|/)assets/(web-dxh[0-9a-f]+\.js)$")
_WASM_RE = re.compile(r"web_bg-dxh[0-9a-f]+\.wasm")


def _local_path(reference: str) -> str | None:
    parsed = urlsplit(reference)
    if parsed.scheme or parsed.netloc or reference.startswith(("data:", "#")):
        return None
    path = unquote(parsed.path).lstrip("/")
    normalized = str(PurePosixPath(path))
    while normalized.startswith("./"):
        normalized = normalized[2:]
    if normalized == ".." or normalized.startswith("../"):
        return None
    return normalized


def verify_web_files(
    index_html: str,
    exists: Callable[[str], bool],
    read_text: Callable[[str], str],
) -> list[str]:
    """Validate index/entry/WASM references against an abstract file collection."""
    errors: list[str] = []
    references = []
    for reference in _HTML_ASSET_RE.findall(index_html):
        path = _local_path(reference)
        if path is not None:
            references.append(path)
    for path in sorted(set(references)):
        if not exists(path):
            errors.append(f"index.html references missing asset: {path}")

    entry_paths = [path for path in references if _ENTRY_JS_RE.search(path)]
    if not entry_paths:
        errors.append("index.html does not reference a hashed Dioxus entry script")
        return errors

    for entry_path in sorted(set(entry_paths)):
        if not exists(entry_path):
            continue
        javascript = read_text(entry_path)
        wasm_names = sorted(set(_WASM_RE.findall(javascript)))
        if not wasm_names:
            errors.append(f"{entry_path} does not reference a hashed WASM module")
            continue
        for wasm_name in wasm_names:
            wasm_path = f"assets/{wasm_name}"
            if not exists(wasm_path):
                errors.append(f"{entry_path} references missing WASM module: assets/{wasm_name}")
    return errors


def verify_web_bundle(root: Path) -> list[str]:
    """Return bundle-integrity errors without mutating the bundle."""
    index = root / "index.html"
    if not index.is_file():
        return ["missing index.html"]
    return verify_web_files(
        index.read_text(encoding="utf-8"),
        lambda path: (root / path).is_file(),
        lambda path: (root / path).read_text(encoding="utf-8"),
    )


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "root",
        nargs="?",
        type=Path,
        default=Path("probing/server/web-assets/public"),
        help="bundle root containing index.html",
    )
    args = parser.parse_args(argv)
    errors = verify_web_bundle(args.root)
    if errors:
        print(f"error: invalid web bundle under {args.root}:", file=sys.stderr)
        for error in errors:
            print(f"  - {error}", file=sys.stderr)
        return 1
    print(f"ok: web bundle references are complete under {args.root}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

"""Client-side API contract helpers (Web UI + CLI).

The canonical HTTP surface is defined in ``api_spec.json`` — not in the Server
crate. These helpers let client code and tests validate against that spec.
"""

from __future__ import annotations

import re
from pathlib import Path
from typing import Any, Iterable

from spec_loader import load_api_spec

# String literals and format! prefixes in Rust client sources.
_STRING_PATH_RE = re.compile(r'"(/(?:apis|query|ws)[^"]*)"')
_FORMAT_PREFIX_RE = re.compile(r'format!\("(/?(?:apis|query|ws)[^"{]*)')


def repo_root() -> Path:
    return Path(__file__).resolve().parents[2]


def normalize_api_path(path: str) -> str:
    """Drop query/fragment/format placeholders; keep rdmaextension trailing slash."""
    path = path.split("?", 1)[0].split("#", 1)[0]
    if "{" in path:
        path = path.split("{", 1)[0]
    if path.startswith("/apis/rdmaextension"):
        return "/apis/rdmaextension/"
    return path.rstrip("/") or path


def canonical_endpoints(spec: dict[str, Any] | None = None) -> list[dict[str, str]]:
    """All first-class HTTP endpoints from the spec."""
    spec = spec or load_api_spec()
    endpoints: list[dict[str, str]] = []

    for entry in spec["server_public"]:
        endpoints.append({"method": entry["method"], "path": entry["path"]})

    ext = spec["routing"]["python_http_extension_name"]
    for handler in spec["pythonext_handlers"]:
        endpoints.append(
            {
                "method": handler["method"],
                "path": f"/apis/{ext}/{handler['local_path']}",
            }
        )

    for entry in spec["other_extensions"]:
        endpoints.append({"method": entry["method"], "path": entry["path"]})

    for entry in spec["top_level"]:
        endpoints.append({"method": entry["method"], "path": entry["path"]})

    return endpoints


def deprecated_paths(spec: dict[str, Any] | None = None) -> set[str]:
    spec = spec or load_api_spec()
    return {item["path"] for item in spec["deprecated_paths"]}


def endpoint_index(spec: dict[str, Any] | None = None) -> dict[str, set[str]]:
    """Map normalized path -> allowed HTTP methods."""
    index: dict[str, set[str]] = {}
    for entry in canonical_endpoints(spec):
        key = normalize_api_path(entry["path"])
        index.setdefault(key, set()).add(entry["method"])
    return index


def path_is_canonical(path: str, spec: dict[str, Any] | None = None) -> bool:
    """True when *path* matches a spec endpoint exactly or as a dynamic prefix."""
    normalized = normalize_api_path(path)
    if normalized in endpoint_index(spec):
        return True
    # Dynamic flamegraph segment: /apis/flamegraph/{torch|pprof}
    if normalized in {"/apis/flamegraph", "/apis/flamegraph/"}:
        return True
    if normalized.startswith("/apis/flamegraph/"):
        suffix = normalized.removeprefix("/apis/flamegraph/").strip("/")
        if suffix in {"torch", "pprof"}:
            return True
    return False


def methods_for_path(path: str, spec: dict[str, Any] | None = None) -> set[str]:
    normalized = normalize_api_path(path)
    return endpoint_index(spec).get(normalized, set())


def extract_api_path_literals(source: str) -> set[str]:
    """Find API path literals embedded in Rust client source."""
    found: set[str] = set()
    for pattern in (_STRING_PATH_RE, _FORMAT_PREFIX_RE):
        for match in pattern.finditer(source):
            raw = match.group(1)
            if not raw.startswith("/"):
                raw = f"/{raw}"
            found.add(normalize_api_path(raw))
    return found


def client_contracts(
    spec: dict[str, Any] | None = None
) -> dict[str, list[dict[str, Any]]]:
    spec = spec or load_api_spec()
    return spec["client_contracts"]


def iter_client_sources(
    client: str, spec: dict[str, Any] | None = None
) -> Iterable[Path]:
    root = repo_root()
    for entry in client_contracts(spec)[client]:
        yield root / entry["source"]


def path_literal_in_source(path: str, source: str) -> bool:
    """Check that a declared contract path is reflected in Rust source."""
    if path in source:
        return True
    if path.startswith("/apis/flamegraph/"):
        return 'format!("/apis/flamegraph/' in source or "/apis/flamegraph/" in source
    normalized = normalize_api_path(path)
    return normalized in source or f"{normalized}?" in source


def declared_client_calls(
    client: str, spec: dict[str, Any] | None = None
) -> list[dict[str, str]]:
    calls: list[dict[str, str]] = []
    for entry in client_contracts(spec)[client]:
        for call in entry["calls"]:
            calls.append(
                {
                    "source": entry["source"],
                    "method": call["method"],
                    "path": call["path"],
                }
            )
    return calls

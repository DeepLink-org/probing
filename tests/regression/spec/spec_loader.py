"""Load the canonical API spec shared by Python contract tests."""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any

SPEC_PATH = Path(__file__).with_name("api_spec.json")


def load_api_spec() -> dict[str, Any]:
    return json.loads(SPEC_PATH.read_text(encoding="utf-8"))


def pythonext_http_paths(spec: dict[str, Any] | None = None) -> list[str]:
    """Full HTTP paths for pythonext handlers."""
    spec = spec or load_api_spec()
    ext = spec["routing"]["python_http_extension_name"]
    return [f"/apis/{ext}/{h['local_path']}" for h in spec["pythonext_handlers"]]


def extension_route_key(extension_name: str, local_path: str) -> str:
    """Key used by server extension response lookup (no leading slash)."""
    local = local_path.strip("/")
    return f"{extension_name}/{local}" if local else extension_name


def pythonext_response_entries(
    spec: dict[str, Any] | None = None,
) -> list[dict[str, Any]]:
    """pythonext handler entries that declare HTTP response metadata."""
    spec = spec or load_api_spec()
    ext = spec["routing"]["python_http_extension_name"]
    entries = []
    for handler in spec["pythonext_handlers"]:
        entries.append(
            {
                "route_key": extension_route_key(ext, handler["local_path"]),
                **handler["response"],
            }
        )
    return entries

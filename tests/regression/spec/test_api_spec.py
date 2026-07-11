"""API contract spec tests.

The machine-readable spec lives in ``tests/regression/spec/api_spec.json``.
These tests fail when implementation drifts from the documented contract.
"""

from __future__ import annotations

import importlib
import json
import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / "python"))
sys.path.insert(0, str(Path(__file__).resolve().parent))

from probing.handlers.router import _handlers, handle_request
from client_contract import (
    declared_client_calls,
    deprecated_paths,
    extract_api_path_literals,
    iter_client_sources,
    normalize_api_path,
    path_is_canonical,
    path_literal_in_source,
    methods_for_path,
    repo_root,
)
from spec_loader import load_api_spec, pythonext_http_paths, pythonext_response_entries


@pytest.fixture(scope="module")
def spec() -> dict:
    return load_api_spec()


@pytest.fixture(scope="module", autouse=True)
def registered_handlers():
    """Ensure pythonext handlers are loaded once for spec tests."""
    import probing.handlers.pythonext  # noqa: F401

    yield
    # Do not clear — other tests may rely on registration order.


class TestSpecFile:
    """Spec file itself stays valid and self-consistent."""

    def test_spec_version(self, spec):
        assert spec["version"] >= 1

    def test_pythonext_paths_are_unique(self, spec):
        paths = [h["local_path"] for h in spec["pythonext_handlers"]]
        assert len(paths) == len(set(paths)), f"duplicate local paths: {paths}"

    def test_server_public_paths_are_unique(self, spec):
        keys = [(r["method"], r["path"]) for r in spec["server_public"]]
        assert len(keys) == len(set(keys))

    def test_deprecated_paths_do_not_overlap_canonical(self, spec):
        canonical = set(pythonext_http_paths(spec))
        canonical.update(r["path"] for r in spec["server_public"])
        for item in spec["deprecated_paths"]:
            assert item["path"] not in canonical

    def test_pythonext_handlers_declare_response_metadata(self, spec):
        allowed_types = {"application/json", "text/plain"}
        for handler in spec["pythonext_handlers"]:
            local = handler["local_path"]
            response = handler.get("response")
            assert response is not None, f"missing response metadata: {local}"
            assert response["content_type"] in allowed_types, local
            assert isinstance(response["cors"], bool), local


class TestPythonextHandlerRegistration:
    """Spec: every pythonext handler is registered exactly once, no aliases."""

    def test_all_spec_handlers_registered(self, spec):
        expected = {h["local_path"] for h in spec["pythonext_handlers"]}
        registered = set(_handlers.keys())
        missing = expected - registered
        extra = registered - expected
        assert not missing, f"missing handlers: {sorted(missing)}"
        assert not extra, (
            f"unexpected handlers (update spec or remove): {sorted(extra)}"
        )

    def test_handler_metadata_matches_spec(self, spec):
        by_path = {h["local_path"]: h for h in spec["pythonext_handlers"]}
        for local_path, entry in by_path.items():
            info = _handlers[local_path]
            assert info.get("uses_body") is entry["uses_body"], local_path
            assert info.get("ext_name") == spec["routing"]["python_http_extension_name"]

    def test_legacy_local_paths_do_not_resolve(self):
        """Local paths with embedded python/ or pythonext/ prefix must not match."""
        legacy_locals = [
            "python/ray/timeline",
            "python/ray/timeline/chrome",
            "python/magics",
            "pythonext/magics",
        ]
        for local in legacy_locals:
            result = json.loads(handle_request(local, {}))
            assert "error" in result, (
                f"legacy path must not resolve: {local!r} -> {result}"
            )


class TestPythonextCanonicalHttpUrls:
    """Spec: HTTP URLs follow /apis/pythonext/{local_path}."""

    def test_canonical_urls_list(self, spec):
        urls = pythonext_http_paths(spec)
        assert "/apis/pythonext/callstack" in urls
        assert "/apis/pythonext/eval" in urls
        assert "/apis/pythonext/ray/timeline" in urls
        assert all("/apis/python/" not in u for u in urls)


class TestExtensionResponseSpec:
    """Spec: extension HTTP response headers are explicit per handler."""

    def test_cors_endpoints_are_timeline_or_chrome_tracing(self, spec):
        cors_paths = {
            entry["route_key"].split("/", 1)[1]
            for entry in pythonext_response_entries(spec)
            if entry["cors"]
        }
        assert cors_paths == {
            "trace/chrome-tracing",
            "pytorch/timeline",
            "ray/timeline",
            "ray/timeline/chrome",
        }

    def test_eval_uses_plain_text_response(self, spec):
        by_path = {h["local_path"]: h for h in spec["pythonext_handlers"]}
        assert by_path["eval"]["response"]["content_type"] == "text/plain"
        assert by_path["eval"]["response"]["cors"] is False


class TestClientContracts:
    """Web UI and CLI validate against api_spec.json — not Server source."""

    @pytest.mark.parametrize("client", ["web", "cli"])
    def test_declared_calls_are_canonical(self, spec, client):
        for call in declared_client_calls(client, spec):
            path = call["path"]
            assert path_is_canonical(path, spec), (
                f"{client} {call['source']} declares non-canonical path {path!r}"
            )
            allowed = methods_for_path(path, spec)
            assert call["method"] in allowed, (
                f"{client} {call['source']}: {call['method']} {path} "
                f"not in spec methods {sorted(allowed)}"
            )

    @pytest.mark.parametrize("client", ["web", "cli"])
    def test_declared_paths_appear_in_source(self, spec, client):
        root = repo_root()
        for call in declared_client_calls(client, spec):
            source = root / call["source"]
            text = source.read_text(encoding="utf-8")
            assert path_literal_in_source(call["path"], text), (
                f"{call['source']} missing path literal for {call['path']!r}"
            )

    @pytest.mark.parametrize("client", ["web", "cli"])
    def test_source_literals_are_not_deprecated(self, spec, client):
        deprecated = deprecated_paths(spec)
        for source in iter_client_sources(client, spec):
            for path in extract_api_path_literals(source.read_text(encoding="utf-8")):
                for bad in deprecated:
                    assert not path.startswith(normalize_api_path(bad)), (
                        f"{source.relative_to(repo_root())} uses deprecated path {path!r} "
                        f"(replacement: {bad})"
                    )

    def test_web_api_modules_do_not_use_legacy_python_prefix(self, spec):
        root = repo_root()
        for source in sorted((root / "web/src/api").glob("*.rs")):
            if source.name == "mod.rs":
                continue
            text = source.read_text(encoding="utf-8")
            assert "/apis/python/" not in text, f"{source} still uses /apis/python/"

    def test_web_extracted_paths_are_canonical(self, spec):
        root = repo_root()
        for source in sorted((root / "web/src/api").glob("*.rs")):
            if source.name == "mod.rs":
                continue
            for path in extract_api_path_literals(source.read_text(encoding="utf-8")):
                assert path_is_canonical(path, spec), (
                    f"{source.relative_to(repo_root())} references unknown path {path!r}"
                )


class TestEvalUsesBody:
    """Spec: eval is POST body handler, callstack is query-param handler."""

    def test_eval_requires_body(self):
        result = json.loads(handle_request("eval", {}))
        assert "Missing request body" in result.get("error", "")

    def test_callstack_accepts_empty_params(self):
        # May error at runtime without probing attached; router must find handler.
        result = handle_request("callstack", {})
        assert "No handler found" not in result


class TestServerPublicRoutes:
    """Spec: server public routes in Rust match api_spec.json."""

    def test_public_api_routes_const_matches_spec(self, spec):
        repo = Path(__file__).resolve().parents[3]
        mod_rs = (repo / "probing/server/src/server/api/mod.rs").read_text(
            encoding="utf-8"
        )
        expected = {
            (entry["method"], entry["path"].removeprefix("/apis"))
            for entry in spec["server_public"]
        }
        import re

        actual = set(re.findall(r'\("([A-Z]+)", "([^"]+)"\)', mod_rs))
        assert actual == expected, (
            f"PUBLIC_API_ROUTES drift: missing={expected - actual}, extra={actual - expected}"
        )


class TestSpecReloadConsistency:
    """Re-importing pythonext must not duplicate or drop handlers."""

    def test_reload_keeps_spec_handlers(self, spec):
        expected = {h["local_path"] for h in spec["pythonext_handlers"]}
        import probing.handlers.pythonext as px

        importlib.reload(px)
        assert expected == set(_handlers.keys())

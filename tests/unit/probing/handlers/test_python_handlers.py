"""Tests for Python extension handlers and router."""

import json

from probing.handlers.pythonext import handle_api_request
from probing.handlers.router import (
    _handlers,
    ext_handler,
    handle_request,
)


class TestHandlerRouter:
    """Test the handler router system."""

    def setup_method(self):
        """Clear handlers before each test."""
        _handlers.clear()

    def test_router_registration(self):
        """Test that handlers can be registered via decorator."""

        @ext_handler("test", "test/path")
        def test_handler(param1: str, param2: int = 10) -> str:
            return json.dumps({"param1": param1, "param2": param2})

        assert "test/path" in _handlers
        handler_info = _handlers["test/path"]
        assert handler_info["function"] == test_handler
        assert "param1" in handler_info["required_params"]
        assert "param2" not in handler_info["required_params"]

    def test_parameter_parsing(self):
        """Test parameter parsing and type conversion."""
        from typing import List

        @ext_handler("test", "test/parse")
        def test_handler(
            str_param: str,
            int_param: int,
            bool_param: bool,
            list_param: List[str],
        ) -> str:
            assert isinstance(list_param, list)
            return json.dumps(
                {
                    "str": str_param,
                    "int": int_param,
                    "bool": bool_param,
                    "list": list_param,
                }
            )

        params = {
            "str_param": "test",
            "int_param": "42",
            "bool_param": "true",
            "list_param": "a,b,c",
        }

        result = handle_request("test/parse", params)
        parsed = json.loads(result)

        assert parsed["str"] == "test"
        assert parsed["int"] == 42
        assert parsed["bool"] is True
        assert parsed["list"] == ["a", "b", "c"]

    def test_missing_required_param(self):
        """Test error handling for missing required parameters."""

        @ext_handler("test", "test/required")
        def test_handler(param1: str) -> str:
            return json.dumps({"param1": param1})

        result = handle_request("test/required", {})
        parsed = json.loads(result)

        assert "error" in parsed
        assert "Missing required parameter" in parsed["error"]

    def test_body_handler(self):
        """Test POST body handlers."""

        @ext_handler("test", "test/eval", uses_body=True)
        def test_eval(code: str) -> str:
            return json.dumps({"code": code})

        result = handle_request("test/eval", {}, body="print(1)")
        parsed = json.loads(result)
        assert parsed["code"] == "print(1)"

        missing = json.loads(handle_request("test/eval", {}))
        assert "Missing request body" in missing["error"]

    def test_optional_parameters(self):
        """Test optional parameter handling."""

        @ext_handler("test", "test/optional")
        def test_handler(
            required: str,
            optional: str = None,
        ) -> str:
            return json.dumps(
                {
                    "required": required,
                    "optional": optional,
                }
            )

        result1 = handle_request(
            "test/optional", {"required": "test", "optional": "value"}
        )
        parsed1 = json.loads(result1)
        assert parsed1["required"] == "test"
        assert parsed1["optional"] == "value"

        result2 = handle_request("test/optional", {"required": "test"})
        parsed2 = json.loads(result2)
        assert parsed2["required"] == "test"
        assert parsed2["optional"] is None

    def test_optional_parameter_uses_function_default(self):
        """Optional query params should fall back to handler defaults."""

        @ext_handler("test", "test/defaults")
        def test_handler(
            required: str,
            engine_type: str = "sglang",
            metrics_path: str = "/engine_metrics",
            count: int = 10,
        ) -> str:
            return json.dumps(
                {
                    "required": required,
                    "engine_type": engine_type,
                    "metrics_path": metrics_path,
                    "count": count,
                }
            )

        parsed = json.loads(handle_request("test/defaults", {"required": "ok"}))
        assert parsed["engine_type"] == "sglang"
        assert parsed["metrics_path"] == "/engine_metrics"
        assert parsed["count"] == 10


class TestUnifiedEntryPoint:
    """Test the unified entry point."""

    def setup_method(self):
        import probing.handlers.pythonext  # noqa: F401

    def test_handle_api_request(self):
        result = handle_api_request("trace/list", {})
        parsed = json.loads(result)
        assert isinstance(parsed, (dict, list))

    def test_handle_api_request_with_params(self):
        result = handle_api_request("trace/variables", {"limit": "10"})
        parsed = json.loads(result)
        assert isinstance(parsed, (dict, list))

    def test_handle_api_request_invalid_path(self):
        result = handle_api_request("invalid/path", {})
        parsed = json.loads(result)
        assert "error" in parsed
        assert "No handler found" in parsed["error"]


class TestHandlerRegistration:
    """Handler registration is covered by tests/regression/spec/test_api_spec.py."""

    def setup_method(self):
        _handlers.clear()
        import importlib

        import probing.handlers.pythonext

        importlib.reload(probing.handlers.pythonext)

    def test_reload_does_not_duplicate_handlers(self):
        import importlib

        import probing.handlers.pythonext

        before = len(_handlers)
        importlib.reload(probing.handlers.pythonext)
        assert len(_handlers) == before


if __name__ == "__main__":
    try:
        import pytest

        pytest.main([__file__, "-v"])
    except ImportError:
        import unittest

        unittest.main()

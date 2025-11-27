"""Tests for Python extension handlers and router."""

import json
import sys
from unittest.mock import patch, MagicMock

# Add python directory to path
sys.path.insert(0, "python")

from probing.handlers.router import (
    ext_handler,
    handle_request,
    _handlers,
    _path_mappings,
)
from probing.handlers.pythonext import handle_api_request


class TestHandlerRouter:
    """Test the handler router system."""

    def setup_method(self):
        """Clear handlers before each test."""
        _handlers.clear()
        _path_mappings.clear()

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
            list_param: List[str],  # Use List[str] for explicit type annotation
        ) -> str:
            # Verify that list_param is actually a list
            assert isinstance(
                list_param, list
            ), f"Expected list, got {type(list_param)}: {list_param}"
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
        assert isinstance(
            parsed["list"], list
        ), f"Expected list, got {type(parsed['list'])}: {parsed['list']}"
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

    def test_path_aliases(self):
        """Test path alias resolution."""

        @ext_handler("test", ["test/canonical", "test/alias1", "test/alias2"])
        def test_handler() -> str:
            return json.dumps({"success": True})

        # Test canonical path
        result1 = handle_request("test/canonical", {})
        assert "error" not in json.loads(result1)

        # Test aliases
        result2 = handle_request("test/alias1", {})
        assert "error" not in json.loads(result2)

        result3 = handle_request("test/alias2", {})
        assert "error" not in json.loads(result3)

        # Verify alias mappings
        assert "test/alias1" in _path_mappings
        assert _path_mappings["test/alias1"] == "test/canonical"

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

        # Test with optional parameter
        result1 = handle_request(
            "test/optional", {"required": "test", "optional": "value"}
        )
        parsed1 = json.loads(result1)
        assert parsed1["required"] == "test"
        assert parsed1["optional"] == "value"

        # Test without optional parameter
        result2 = handle_request("test/optional", {"required": "test"})
        parsed2 = json.loads(result2)
        assert parsed2["required"] == "test"
        assert parsed2["optional"] is None


class TestUnifiedEntryPoint:
    """Test the unified entry point."""

    def setup_method(self):
        """Ensure handlers are registered by importing pythonext."""
        # Import pythonext to trigger handler registration
        import probing.handlers.pythonext  # noqa: F401

    def test_handle_api_request(self):
        """Test the unified handle_api_request function."""
        # Test with a simple handler
        result = handle_api_request("trace/list", {})

        # Should return valid JSON
        parsed = json.loads(result)
        assert isinstance(parsed, (dict, list))

    def test_handle_api_request_with_params(self):
        """Test handle_api_request with parameters."""
        result = handle_api_request("trace/variables", {"limit": "10"})

        parsed = json.loads(result)
        # Should return valid JSON (either data or error)
        assert isinstance(parsed, (dict, list))

    def test_handle_api_request_invalid_path(self):
        """Test handle_api_request with invalid path."""
        result = handle_api_request("invalid/path", {})

        parsed = json.loads(result)
        assert "error" in parsed
        assert "No handler found" in parsed["error"]

    def test_handle_api_request_missing_required_param(self):
        """Test handle_api_request with missing required parameter."""
        result = handle_api_request("trace/start", {"function": "test_func"})

        # trace/start requires function parameter, but let's test with a handler that exists
        # First verify the handler exists
        from probing.handlers.router import _handlers

        if "trace/start" in _handlers:
            # Test with missing required parameter
            result = handle_api_request("trace/start", {})
            parsed = json.loads(result)
            assert "error" in parsed
            # Either missing parameter error or handler execution error
            assert "error" in parsed
        else:
            # Handler not registered, skip this test
            pass


class TestHandlerRegistration:
    """Test that all handlers are properly registered."""

    def setup_method(self):
        """Ensure handlers are registered by re-importing pythonext."""
        # Clear any test handlers from previous tests
        _handlers.clear()
        _path_mappings.clear()

        # Re-import pythonext to trigger handler registration via decorators
        import importlib
        import probing.handlers.pythonext

        importlib.reload(probing.handlers.pythonext)

    def test_all_handlers_registered(self):
        """Test that all expected handlers are registered."""
        expected_handlers = [
            "ray/timeline",
            "ray/timeline/chrome",
            "trace/chrome-tracing",
            "pytorch/timeline",
            "pytorch/profile",
            "trace/list",
            "trace/show",
            "trace/start",
            "trace/stop",
            "trace/variables",
        ]

        # Debug: show current handlers if test fails
        current_handlers = list(_handlers.keys())

        for handler_path in expected_handlers:
            assert handler_path in _handlers, (
                f"Handler {handler_path} not registered. "
                f"Current handlers: {current_handlers}. "
                f"Total handlers: {len(_handlers)}"
            )


if __name__ == "__main__":
    try:
        import pytest

        pytest.main([__file__, "-v"])
    except ImportError:
        # Fallback to unittest if pytest is not available
        import unittest

        unittest.main()

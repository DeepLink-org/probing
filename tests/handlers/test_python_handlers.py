"""Tests for Python extension handlers and router."""

import json
import sys
from unittest.mock import patch, MagicMock

# Add python directory to path
sys.path.insert(0, "python")

from probing.handlers.router import HandlerRouter, ParamType, handle_request
from probing.handlers.pythonext import handle_api_request


class TestHandlerRouter:
    """Test the handler router system."""
    
    def test_router_registration(self):
        """Test that handlers can be registered."""
        router = HandlerRouter()
        
        def test_handler(param1: str, param2: int = 10) -> str:
            return json.dumps({"param1": param1, "param2": param2})
        
        router.register(
            "test/path",
            test_handler,
            required_params=["param1"],
            param_types={
                "param1": ParamType.STRING,
                "param2": ParamType.INT,
            },
        )
        
        assert "test/path" in router._handlers
    
    def test_parameter_parsing(self):
        """Test parameter parsing and type conversion."""
        router = HandlerRouter()
        
        def test_handler(
            str_param: str,
            int_param: int,
            bool_param: bool,
            list_param: list,
        ) -> str:
            return json.dumps({
                "str": str_param,
                "int": int_param,
                "bool": bool_param,
                "list": list_param,
            })
        
        router.register(
            "test/parse",
            test_handler,
            required_params=["str_param", "int_param"],
            param_types={
                "str_param": ParamType.STRING,
                "int_param": ParamType.INT,
                "bool_param": ParamType.BOOL,
                "list_param": ParamType.STRING_LIST,
            },
        )
        
        params = {
            "str_param": "test",
            "int_param": "42",
            "bool_param": "true",
            "list_param": "a,b,c",
        }
        
        result = router.handle("test/parse", params)
        parsed = json.loads(result)
        
        assert parsed["str"] == "test"
        assert parsed["int"] == 42
        assert parsed["bool"] is True
        assert parsed["list"] == ["a", "b", "c"]
    
    def test_missing_required_param(self):
        """Test error handling for missing required parameters."""
        router = HandlerRouter()
        
        def test_handler(param1: str) -> str:
            return json.dumps({"param1": param1})
        
        router.register(
            "test/required",
            test_handler,
            required_params=["param1"],
            param_types={"param1": ParamType.STRING},
        )
        
        result = router.handle("test/required", {})
        parsed = json.loads(result)
        
        assert "error" in parsed
        assert "Missing required parameter" in parsed["error"]
    
    def test_path_aliases(self):
        """Test path alias resolution."""
        router = HandlerRouter()
        
        def test_handler() -> str:
            return json.dumps({"success": True})
        
        router.register(
            "test/canonical",
            test_handler,
            path_aliases=["test/alias1", "test/alias2"],
        )
        
        # Test canonical path
        result1 = router.handle("test/canonical", {})
        assert "error" not in json.loads(result1)
        
        # Test aliases
        result2 = router.handle("test/alias1", {})
        assert "error" not in json.loads(result2)
        
        result3 = router.handle("test/alias2", {})
        assert "error" not in json.loads(result3)
    
    def test_optional_parameters(self):
        """Test optional parameter handling."""
        router = HandlerRouter()
        
        def test_handler(
            required: str,
            optional: str = None,
        ) -> str:
            return json.dumps({
                "required": required,
                "optional": optional,
            })
        
        router.register(
            "test/optional",
            test_handler,
            required_params=["required"],
            param_types={
                "required": ParamType.STRING,
                "optional": ParamType.OPTIONAL_STRING,
            },
        )
        
        # Test with optional parameter
        result1 = router.handle("test/optional", {"required": "test", "optional": "value"})
        parsed1 = json.loads(result1)
        assert parsed1["required"] == "test"
        assert parsed1["optional"] == "value"
        
        # Test without optional parameter
        result2 = router.handle("test/optional", {"required": "test"})
        parsed2 = json.loads(result2)
        assert parsed2["required"] == "test"
        assert parsed2["optional"] is None


class TestUnifiedEntryPoint:
    """Test the unified entry point."""
    
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
        result = handle_api_request("trace/start", {})
        
        parsed = json.loads(result)
        assert "error" in parsed
        assert "Missing required parameter" in parsed["error"]


class TestHandlerRegistration:
    """Test that all handlers are properly registered."""
    
    def test_all_handlers_registered(self):
        """Test that all expected handlers are registered."""
        from probing.handlers.router import get_router
        
        router = get_router()
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
        
        for handler_path in expected_handlers:
            assert handler_path in router._handlers, f"Handler {handler_path} not registered"


if __name__ == "__main__":
    import pytest
    pytest.main([__file__, "-v"])


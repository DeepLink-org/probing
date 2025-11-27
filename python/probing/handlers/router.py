"""Unified handler router for Python extension API endpoints.

This module provides a centralized routing system that maps API paths to handler functions
with automatic parameter parsing and validation.
"""

import json
import traceback
import inspect
from typing import Optional, Dict, Any, Callable, List, Tuple, Union


# Global router state
_handlers: Dict[str, Dict[str, Any]] = (
    {}
)  # path -> {function, required_params, param_types}
_path_mappings: Dict[str, str] = {}  # alias -> canonical path


def _normalize_path(path: str) -> str:
    """Normalize path by removing leading slashes and resolving aliases.

    >>> # Clean up and set up test data
    >>> _handlers.clear()
    >>> _path_mappings.clear()
    >>> _handlers["test/path"] = {"function": lambda: None, "required_params": [], "param_types": {}}
    >>> _path_mappings["test/alias"] = "test/path"
    >>>
    >>> # Test direct match
    >>> _normalize_path("test/path")
    'test/path'
    >>>
    >>> # Test alias resolution
    >>> _normalize_path("test/alias")
    'test/path'
    >>>
    >>> # Test leading slash removal
    >>> _normalize_path("/test/path")
    'test/path'
    >>>
    >>> # Test unknown path returns normalized
    >>> _normalize_path("unknown/path")
    'unknown/path'
    """
    normalized = path.strip("/")

    if normalized in _handlers:
        return normalized

    if normalized in _path_mappings:
        return _path_mappings[normalized]

    # Check for suffix match
    for canonical_path in _handlers:
        if normalized.endswith(canonical_path) or canonical_path.endswith(normalized):
            return canonical_path

    return normalized


def _parse_param(value: str, param_type: str) -> Any:
    """Parse a parameter value according to its type.

    >>> _parse_param("hello", "string")
    'hello'
    >>> _parse_param("42", "int")
    42
    >>> _parse_param("true", "bool")
    True
    >>> _parse_param("false", "bool")
    False
    >>> _parse_param("a,b,c", "string_list")
    ['a', 'b', 'c']
    >>> _parse_param("", "optional_string") is None
    True
    >>> _parse_param("test", "optional_string")
    'test'
    >>> _parse_param("", "optional_int") is None
    True
    >>> _parse_param("123", "optional_int")
    123
    """
    if param_type == "string":
        return value
    elif param_type in ("int", "i64", "usize"):
        return int(value)
    elif param_type == "bool":
        return value.lower() in ("true", "1", "yes", "on")
    elif param_type == "string_list":
        return [s.strip() for s in value.split(",") if s.strip()]
    elif param_type == "optional_string":
        return value if value else None
    elif param_type in ("optional_int", "optional_i64"):
        return int(value) if value else None
    else:
        return value


def _parse_params(
    params: Dict[str, str], handler_info: Dict[str, Any]
) -> Tuple[Dict[str, Any], Optional[str]]:
    """Parse and validate parameters.

    >>> # Test successful parsing
    >>> handler_info = {
    ...     "required_params": ["name"],
    ...     "param_types": {"name": "string", "age": "int", "active": "bool"}
    ... }
    >>> params = {"name": "Alice", "age": "25", "active": "true"}
    >>> parsed, error = _parse_params(params, handler_info)
    >>> error is None
    True
    >>> parsed["name"]
    'Alice'
    >>> parsed["age"]
    25
    >>> parsed["active"]
    True
    >>>
    >>> # Test missing required parameter
    >>> params = {"age": "25"}
    >>> parsed, error = _parse_params(params, handler_info)
    >>> error is not None
    True
    >>> "Missing required parameter" in error
    True
    >>>
    >>> # Test optional parameters
    >>> handler_info = {
    ...     "required_params": [],
    ...     "param_types": {"value": "optional_string", "count": "optional_int"}
    ... }
    >>> params = {}
    >>> parsed, error = _parse_params(params, handler_info)
    >>> error is None
    True
    >>> parsed["value"] is None
    True
    >>> parsed["count"] is None
    True
    >>>
    >>> # Test invalid parameter type
    >>> params = {"count": "not_a_number"}
    >>> parsed, error = _parse_params(params, handler_info)
    >>> error is not None
    True
    >>> "Invalid parameter" in error
    True
    """
    parsed = {}
    required_params = handler_info.get("required_params", [])
    param_types = handler_info.get("param_types", {})

    # Check required parameters
    for param_name in required_params:
        if param_name not in params:
            return {}, f"Missing required parameter: {param_name}"

    # Parse all parameters
    for param_name, param_type in param_types.items():
        if param_name in params:
            try:
                parsed[param_name] = _parse_param(params[param_name], param_type)
            except (ValueError, TypeError) as e:
                return {}, f"Invalid parameter '{param_name}': {str(e)}"
        elif param_type in ("optional_string", "optional_int", "optional_i64"):
            parsed[param_name] = None
        elif param_name in required_params:
            return {}, f"Missing required parameter: {param_name}"

    return parsed, None


def handle_request(path: str, params: Dict[str, str]) -> str:
    """Handle a request using the global router.

    Args:
        path: Request path
        params: Query parameters as string dictionary

    Returns:
        JSON string response

    Example:
        >>> # Clean up and register a test handler
        >>> _handlers.clear()
        >>> _path_mappings.clear()
        >>> @ext_handler("test", "test/example")
        ... def test_handler(name: str, age: int) -> str:
        ...     return json.dumps({"name": name, "age": age})
        >>>
        >>> # Call the handler
        >>> result = handle_request("test/example", {"name": "Alice", "age": "25"})
        >>> import json
        >>> data = json.loads(result)
        >>> data["name"]
        'Alice'
        >>> data["age"]
        25
        >>>
        >>> # Test error handling - missing required parameter
        >>> result = handle_request("test/example", {"name": "Bob"})
        >>> "error" in result
        True
        >>>
        >>> # Test error handling - nonexistent path
        >>> result = handle_request("nonexistent/path", {})
        >>> "error" in result
        True
    """
    try:
        normalized_path = _normalize_path(path)

        if normalized_path not in _handlers:
            return json.dumps(
                {
                    "error": f"No handler found for path: {path}",
                    "normalized_path": normalized_path,
                }
            )

        handler_info = _handlers[normalized_path]
        parsed_params, error = _parse_params(params, handler_info)

        if error:
            return json.dumps({"error": error})

        try:
            result = handler_info["function"](**parsed_params)
            return result if isinstance(result, str) else json.dumps(result)
        except Exception as e:
            return json.dumps(
                {
                    "error": str(e),
                    "traceback": traceback.format_exc(),
                }
            )
    except Exception as e:
        return json.dumps(
            {
                "error": f"Router error: {str(e)}",
                "traceback": traceback.format_exc(),
            }
        )


def _infer_param_types(func: Callable) -> Dict[str, str]:
    """Infer parameter types from function signature.

    >>> from typing import Optional, List
    >>> def test_func(name: str, age: int, active: bool = True):
    ...     pass
    >>> types = _infer_param_types(test_func)
    >>> types["name"]
    'string'
    >>> types["age"]
    'int'
    >>> types["active"]
    'bool'

    >>> def test_optional(value: Optional[str] = None, count: Optional[int] = None):
    ...     pass
    >>> types = _infer_param_types(test_optional)
    >>> types["value"]
    'optional_string'
    >>> types["count"]
    'optional_int'

    >>> def test_list(items: List[str]):
    ...     pass
    >>> types = _infer_param_types(test_list)
    >>> types["items"]
    'string_list'
    """
    import typing

    sig = inspect.signature(func)
    param_types = {}

    for param_name, param in sig.parameters.items():
        if param_name == "self":
            continue

        param_type = param.annotation
        has_default = param.default != inspect.Parameter.empty

        # Handle typing.Optional and Union[..., None]
        if hasattr(param_type, "__origin__"):
            origin = param_type.__origin__

            # Handle Optional[Type] which is Union[Type, None]
            if origin is typing.Union or (
                hasattr(typing, "Union") and origin is typing.Union
            ):
                args = getattr(param_type, "__args__", ())
                # Check if it's Optional (Union[Type, None] or Union[None, Type])
                if len(args) == 2 and type(None) in args:
                    inner_type = next(t for t in args if t is not type(None))
                    if inner_type == str:
                        param_types[param_name] = "optional_string"
                    elif inner_type == int:
                        param_types[param_name] = "optional_int"
                    elif (
                        hasattr(inner_type, "__origin__")
                        and inner_type.__origin__ is list
                    ):
                        param_types[param_name] = "string_list"
                    else:
                        param_types[param_name] = "optional_string"
                    continue

            # Handle List[Type]
            if origin is list or (hasattr(typing, "List") and origin is typing.List):
                param_types[param_name] = "string_list"
                continue

        # Handle basic types
        if param_type == str:
            param_types[param_name] = "optional_string" if has_default else "string"
        elif param_type == int:
            param_types[param_name] = "optional_int" if has_default else "int"
        elif param_type == bool:
            param_types[param_name] = "bool"
        elif has_default:
            # Has default value but unknown type, treat as optional string
            param_types[param_name] = "optional_string"
        else:
            # No type annotation and no default, treat as string
            param_types[param_name] = "string"

    return param_types


def _infer_required_params(func: Callable) -> List[str]:
    """Infer required parameters from function signature.

    >>> def test_func(name: str, age: int, active: bool = True):
    ...     pass
    >>> _infer_required_params(test_func)
    ['name', 'age']

    >>> def test_all_optional(value: str = "default", count: int = 0):
    ...     pass
    >>> _infer_required_params(test_all_optional)
    []

    >>> def test_no_defaults(a: str, b: int):
    ...     pass
    >>> sorted(_infer_required_params(test_no_defaults))
    ['a', 'b']
    """
    sig = inspect.signature(func)
    required = []

    for param_name, param in sig.parameters.items():
        if param_name == "self":
            continue
        if param.default == inspect.Parameter.empty:
            required.append(param_name)

    return required


def ext_handler(
    ext_name: str,
    paths: Union[str, List[str]],
    required_params: Optional[List[str]] = None,
):
    """Decorator for registering extension handlers.

    Usage:
        @ext_handler("pythonext", "trace/list")
        def list_trace(prefix: Optional[str] = None) -> str:
            ...

        @ext_handler("pythonext", ["ray/timeline", "python/ray/timeline"])
        def get_ray_timeline(...) -> str:
            ...

    Args:
        ext_name: Extension name (e.g., "pythonext")
        paths: Single path string or list of equivalent paths
        required_params: List of required parameter names (auto-inferred if not provided)

    Example:
        >>> # Clean up and register a simple handler
        >>> _handlers.clear()
        >>> _path_mappings.clear()
        >>> @ext_handler("test", "test/simple")
        ... def simple_handler() -> str:
        ...     return json.dumps({"result": "ok"})
        >>>
        >>> # Check it's registered
        >>> "test/simple" in _handlers
        True
        >>>
        >>> # Register with path aliases
        >>> @ext_handler("test", ["test/main", "test/alt"])
        ... def aliased_handler() -> str:
        ...     return json.dumps({"result": "aliased"})
        >>>
        >>> # Check aliases are registered
        >>> "test/main" in _handlers
        True
        >>> "test/alt" in _path_mappings
        True
        >>> _path_mappings["test/alt"]
        'test/main'
        >>>
        >>> # Register with required params
        >>> @ext_handler("test", "test/required", required_params=["id"])
        ... def required_handler(id: str) -> str:
        ...     return json.dumps({"id": id})
        >>>
        >>> handler_info = _handlers["test/required"]
        >>> "id" in handler_info["required_params"]
        True
    """

    def decorator(func: Callable) -> Callable:
        path_list = [paths] if isinstance(paths, str) else paths

        if not path_list:
            raise ValueError("At least one path must be provided")

        canonical_path = path_list[0]
        path_aliases = path_list[1:] if len(path_list) > 1 else []

        # Auto-infer parameter types and required params
        param_types = _infer_param_types(func)
        required = required_params or _infer_required_params(func)

        # Register handler
        _handlers[canonical_path] = {
            "function": func,
            "required_params": required,
            "param_types": param_types,
        }

        # Register aliases
        for alias in path_aliases:
            _path_mappings[alias] = canonical_path

        return func

    return decorator

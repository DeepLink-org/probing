"""
Probing - Dynamic Performance Profiler for Distributed AI

This module provides performance profiling and debugging capabilities for AI applications.
"""

__all__ = [
    "query",
    "load_extension",
    "span",
    "event",
    "cli_main",
    "ExternalTable",
    "TCPStore",
    "VERSION",
    "get_current_script_name",
    "should_enable_probing",
]

VERSION = "0.2.2"

# Import the Rust extension module
# This will trigger the #[pymodule] initialization which registers all functions in _core
try:
    from probing import _core
    _library_loaded = True
except ImportError:
    _library_loaded = False
    _core = None


def get_current_script_name():
    """Get the name of the current running script."""
    import sys
    import os

    try:
        script_path = sys.argv[0]
        return os.path.basename(script_path)
    except (IndexError, AttributeError):
        return "<unknown>"


def should_enable_probing():
    """
    Check if probing should be enabled based on PROBING environment variable.
    Uses the same logic as python/probing_hook.py.

    Returns:
        bool: True if probing should be enabled, False otherwise.
    """
    import os
    import sys

    # Get the PROBING environment variable
    # Check PROBING_ORIGINAL first (saved by probing_hook.py before deletion)
    # then fall back to PROBING
    probe_value = os.environ.get("PROBING_ORIGINAL") or os.environ.get("PROBING", "0")

    # If set to "0", disabled
    if probe_value == "0":
        return False

    # Handle init: prefix (extract the probe setting part)
    if probe_value.startswith("init:"):
        parts = probe_value.split("+", 1)
        probe_value = parts[1] if len(parts) > 1 else "0"
        # Note: init script execution is handled by probing_hook.py, not here

    # Handle "1" or "followed" - enable in current process
    if probe_value.lower() in ["1", "followed"]:
        return True

    # Handle "2" or "nested" - enable in current and child processes
    if probe_value.lower() in ["2", "nested"]:
        return True

    # Handle regex: pattern
    if probe_value.lower().startswith("regex:"):
        pattern = probe_value.split(":", 1)[1]
        try:
            import re

            current_script = get_current_script_name()
            return re.search(pattern, current_script) is not None
        except Exception:
            # If regex is invalid, don't enable
            return False

    # Handle script name matching
    current_script = get_current_script_name()
    if probe_value == current_script:
        return True

    # Default: don't enable if value doesn't match any pattern
    return False


# Import functions from Rust module or provide dummy implementations
if _library_loaded:
    # First, export ExternalTable and TCPStore from _core to probing module namespace
    # This must be done before importing other modules that might use ExternalTable
    # (e.g., probing.inspect.trace uses @table decorator which accesses probing.ExternalTable)
    ExternalTable = _core.ExternalTable
    TCPStore = _core.TCPStore
    
    # Export config and _tracing modules from _core
    # These are registered as attributes in _core, not submodules
    config = _core.config
    _tracing = _core._tracing
    
    # Now import other modules that may depend on ExternalTable
    import probing.hooks.import_hook
    import probing.inspect

    from probing.core.engine import query
    from probing.core.engine import load_extension

    from probing.tracing import span
    from probing.tracing import event

    # Import cli_main directly from _core module
    # All Rust functions are registered in _core via #[pymodule]
    cli_main = _core.cli_main
else:
    # Dummy mode - library not loaded
    import functools

    # Provide dummy classes for ExternalTable and TCPStore
    class ExternalTable:
        def __init__(self, *args, **kwargs):
            raise ImportError("Probing library is not loaded. Please install the probing package.")
        
        @classmethod
        def get(cls, *args, **kwargs):
            raise ImportError("Probing library is not loaded. Please install the probing package.")
    
    class TCPStore:
        def __init__(self, *args, **kwargs):
            raise ImportError("Probing library is not loaded. Please install the probing package.")

    def query(*args, **kwargs):
        raise ImportError("Probing library is not loaded. Please install the probing package.")

    def load_extension(*args, **kwargs):
        raise ImportError("Probing library is not loaded. Please install the probing package.")

    def span(*args, **kwargs):
        """Dummy span implementation."""
        if len(args) == 0 and not kwargs:
            def decorator(func):
                @functools.wraps(func)
                def wrapper(*wargs, **wkwargs):
                    return func(*wargs, **wkwargs)
                return wrapper
            return decorator

        if len(args) == 1 and callable(args[0]):
            func = args[0]
            @functools.wraps(func)
            def wrapper(*wargs, **wkwargs):
                return func(*wargs, **wkwargs)
            return wrapper

        if len(args) == 1 and isinstance(args[0], str):
            name = args[0]
            class DummySpanWrapper:
                def __init__(self, name: str, **attrs):
                    self.name = name
                    self.attrs = attrs
                def __call__(self, func):
                    @functools.wraps(func)
                    def wrapper(*wargs, **wkwargs):
                        return func(*wargs, **wkwargs)
                    return wrapper
                def __enter__(self):
                    return self
                def __exit__(self, *exc):
                    return False
            return DummySpanWrapper(name, **kwargs)

        if len(args) > 0:
            name = args[0]
            if not isinstance(name, str):
                raise TypeError("span() requires a string name as the first argument")
            class DummySpan:
                def __init__(self, name: str, **attrs):
                    self.name = name
                    self.attrs = attrs
                def __enter__(self):
                    return self
                def __exit__(self, *exc):
                    return False
            return DummySpan(name, **kwargs)

        raise TypeError("span() requires at least one argument")

    def event(*args, **kwargs):
        return

    def cli_main(*args, **kwargs):
        raise ImportError("Probing library is not loaded. Please install the probing package.")

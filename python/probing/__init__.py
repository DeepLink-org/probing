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
    "config",
    "enable_tracer",
    "disable_tracer",
    "_get_python_stacks",
    "_get_python_frames",
    "VERSION",
    "get_current_script_name",
    "should_enable_probing",
    "is_enabled",
]

VERSION = "0.2.2"

# Import the Rust extension module
# This will trigger the #[pymodule] initialization which registers all functions in _core
from probing import _core


def get_current_script_name():
    """Get the name of the current running script."""
    import sys
    import os

    try:
        script_path = sys.argv[0]
        return os.path.basename(script_path)
    except (IndexError, AttributeError):
        return "<unknown>"


def is_enabled():
    """
    Check if probing is currently enabled in the backend.
    
    Returns:
        bool: True if probing is enabled, False otherwise.
    """
    try:
        return _core.is_enabled()
    except AttributeError:
        # Fallback if _core doesn't have is_enabled yet (e.g. partial build)
        return should_enable_probing()

def should_enable_probing():
    """
    Check if probing should be enabled based on PROBING environment variable.
    Uses the same logic as python/probing_hook.py.

    Returns:
        bool: True if probing should be enabled, False otherwise.
    """
    # Use the implementation from _core which handles PROBING and PROBING_ORIGINAL
    # consistently with the Rust setup logic
    return _core.should_enable_probing()


# Export ExternalTable and TCPStore from _core to probing module namespace
# This must be done before importing other modules that might use ExternalTable
# (e.g., probing.inspect.trace uses @table decorator which accesses probing.ExternalTable)
ExternalTable = _core.ExternalTable
TCPStore = _core.TCPStore

# Import config module (which wraps _core functions)
import probing.config as config

# _tracing is now flattened into _core, but we keep the internal reference if needed
# or we can remove this alias if it's not used outside of tracing.py
# tracing.py now imports directly from _core
_tracing = _core

# Export functions from _core
enable_tracer = _core.enable_tracer
disable_tracer = _core.disable_tracer
_get_python_stacks = _core._get_python_stacks
_get_python_frames = _core._get_python_frames

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

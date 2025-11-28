"""Handlers for Python extension API endpoints.

This package contains handler functions for various API endpoints.
Handlers are automatically registered via the @ext_handler decorator.
"""

# Import pythonext to trigger handler registration via decorators
import probing.handlers.pythonext  # noqa: F401

# Router system exports
from probing.handlers.router import (
    ext_handler,
    handle_request,
)

__all__ = [
    "ext_handler",
    "handle_request",
]

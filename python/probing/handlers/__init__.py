"""
API Handlers

Spec
----
This module provides the routing and handling logic for Python-based API extensions.

Responsibilities:
1.  Registry for extension handlers via `@ext_handler`.
2.  Dispatch mechanism for incoming requests (`handle_request`).

Public Interfaces:
- `ext_handler`: Decorator to register a function as an API endpoint.
- `handle_request`: Function to process incoming requests and route them to handlers.
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

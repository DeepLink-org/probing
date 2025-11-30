"""
Probing - Dynamic Performance Profiler for Distributed AI

Spec
----
This is the top-level package for the Probing library. It serves as the main entry point
for users and integrations.

Responsibilities:
1.  Export core primitives for operating probing, including `query` and `load_extension`.
2.  Export control functions (enable/disable tracer, CLI main) for runtime management.
3.  Export high-level APIs for tracing (span, event) and engine queries.
4.  Initialize configuration and environment settings.

Public Interfaces:
- Engine: `query`, `load_extension`
- Control: `cli_main`, `enable_tracer`, `disable_tracer`, `is_enabled`
- Tracing: `span`, `event`
- Engine: `query`, `load_extension`
"""

import probing.config as config
from probing import _core

VERSION = "0.2.2"

# Core Primitives
ExternalTable = _core.ExternalTable
TCPStore = _core.TCPStore

# Control Functions
cli_main = _core.cli_main
enable_tracer = _core.enable_tracer
disable_tracer = _core.disable_tracer


def is_enabled():
    return _core.is_enabled()


# Internal Accessors
_get_python_stacks = _core._get_python_stacks
_get_python_frames = _core._get_python_frames

# Submodules with side effects (must be imported after Core Primitives)
from probing.core.engine import load_extension, query
from probing.tracing import event, span

__all__ = [
    "VERSION",
    "ExternalTable",
    "TCPStore",
    "config",
    "cli_main",
    "enable_tracer",
    "disable_tracer",
    "is_enabled",
    "query",
    "load_extension",
    "span",
    "event",
]

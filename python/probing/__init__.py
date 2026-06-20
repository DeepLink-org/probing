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

Pulsing integration is passive: when another runtime writes ``pulsing.*`` memtables
under the shared data directory, probing exposes them as SQL tables. Probing does not
discover, bootstrap, or sync Pulsing cluster membership.
"""

from __future__ import annotations

from probing._entrypoint import is_lightweight_module
from probing.web_assets import configure_assets_root

VERSION = "0.2.5"


if is_lightweight_module():
    __all__ = ["VERSION"]
else:
    configure_assets_root()
    import probing.config as config
    from probing import _core
    from probing.external_table import ExternalTable

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
    from probing.parallel import clear_role, current_role, set_role
    from probing.tracing import event, span

    try:
        from probing.nccl.mock import maybe_auto_seed

        if maybe_auto_seed():
            import logging

            logging.getLogger(__name__).debug(
                "seeded mock nccl.proxy_ops / nccl.net_qp (PROBING_NCCL_MOCK)"
            )
    except Exception:
        pass

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
        "set_role",
        "clear_role",
        "current_role",
    ]

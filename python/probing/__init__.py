"""
Probing - Dynamic Performance Profiler for Distributed AI

Spec
----
This is the top-level package for the main entry point for users and integrations.

Public Interfaces (full import):
- Engine: `query`, `load_extension`
- Control: `cli_main`, `enable_tracer`, `disable_tracer`, `is_enabled`
- Tracing: `span`, `event`, `record_span`, `step`
"""

from __future__ import annotations

from probing._entrypoint import (
    is_elastic_supervisor,
    is_lightweight_module,
    is_probing_cli,
)

VERSION = "0.2.5"

if is_lightweight_module() or is_elastic_supervisor():
    __all__ = ["VERSION"]
elif is_probing_cli():
    # Must set PROBING_CLI_MODE before loading _core so #[ctor] skips engine startup.
    from probing import _core

    cli_main = _core.cli_main
    __all__ = ["VERSION", "cli_main"]
else:
    from probing.web_assets import configure_assets_root

    configure_assets_root()
    import probing.config as config
    from probing import _core
    from probing.external_table import ExternalTable

    TCPStore = _core.TCPStore

    cli_main = _core.cli_main
    enable_tracer = _core.enable_tracer
    disable_tracer = _core.disable_tracer

    def is_enabled():
        return _core.is_enabled()

    _get_python_stacks = _core._get_python_stacks
    _get_python_frames = _core._get_python_frames
    register_table_docs = _core.register_table_docs

    from probing.core.engine import load_extension, query
    from probing.parallel import clear_role, current_role, set_role
    from probing.tracing import (
        attach_training_phases,
        event,
        owns_training_phases,
        phase,
        record_span,
        span,
        step,
    )

    try:
        from probing.nccl.mock import maybe_auto_seed

        if maybe_auto_seed():
            import logging

            logging.getLogger(__name__).debug(
                "seeded mock nccl.proxy_ops / nccl.net_qp (PROBING_NCCL_MOCK)"
            )
    except Exception:
        pass

    try:
        from probing._entrypoint import should_activate_probing

        if should_activate_probing():
            from probing.crash import install

            install()
    except Exception:
        pass

    try:
        from probing.hooks.import_hook import install_and_run_pending

        install_and_run_pending()
    except ImportError:
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
        "record_span",
        "step",
        "phase",
        "attach_training_phases",
        "owns_training_phases",
        "set_role",
        "clear_role",
        "current_role",
    ]

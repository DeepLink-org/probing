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

VERSION = "0.2.4"

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

# 后台探测通过 import_hook 的 register 机制触发（见 add_module_callback）
_detection_started = False


def _start_background_detection() -> None:
    global _detection_started
    import os

    if os.environ.get("PROBING_PULSING_DISCOVER", "1").strip().lower() in (
        "0",
        "false",
        "no",
    ):
        return
    if _detection_started:
        return
    _detection_started = True
    import threading
    import time

    def _run() -> None:
        time.sleep(1.0)
        try:
            from probing import cluster

            cluster.start_pulsing_sync()
        except Exception:  # noqa: S110
            pass

    t = threading.Thread(target=_run, name="probing-background-detection", daemon=True)
    t.start()


def _install_detection_hooks() -> None:
    from probing.hooks import import_hook

    for module_name in import_hook.register:
        import_hook.add_module_callback(module_name, _start_background_detection)


_install_detection_hooks()


def _install_pulsing_hooks() -> None:
    """On ``import pulsing``: cluster node sync + Pulsing ExternalTable refresh thread."""

    def _on_pulsing_cluster() -> None:
        try:
            from probing import cluster

            cluster.start_pulsing_sync()
        except Exception:  # noqa: S110
            pass

    def _on_pulsing_tables() -> None:
        try:
            from pulsing.integrations.probing import start_probing_integration

            start_probing_integration()
        except Exception:  # noqa: S110
            pass

    from probing.hooks import import_hook

    import_hook.add_module_callback("pulsing", _on_pulsing_cluster)
    import_hook.add_module_callback("pulsing", _on_pulsing_tables)


_install_pulsing_hooks()

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

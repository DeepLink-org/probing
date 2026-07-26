"""``probing.fakes`` — invent missing CUDA/Megatron packages for macOS debugging.

Opt-in only. Typical usage::

    from probing.fakes import install
    install(force=True)
    from probing.fakes import run_scripted_loop
    run_scripted_loop(steps=4)

Or via env / CLI::

    PROBING_FAKES=1 PROBING_FAKES_FORCE=1 PROBING_FAKE_DEVICE=meta \\
      python -m probing.fakes loop

Real Megatron-LM checkout (default ``../Megatron-LM``)::

    python examples/megatron/run_megatron_lm_pretrain.py --train-iters 4

**Limits:** ``meta`` has no storage — real forward/backward will fail. This
package is for topology, import, and probing observability debugging.
"""

from __future__ import annotations

import logging
import os
from typing import Iterable, Optional

from . import registry
from .device import (
    install_device_remap,
    is_device_remap_installed,
    resolve_fake_device,
    target_device,
    uninstall_device_remap,
)
from .finder import (
    clear_injected_modules,
    force_enabled,
    install_finder,
    purge_prefixed_modules,
    set_force,
    uninstall_finder,
)
from .torch_hooks import (
    install_torch_dist_hooks,
    uninstall_torch_dist_hooks,
)

# Register built-in specs at import time (no journal / tracing import).
from . import specs as _specs  # noqa: F401

logger = logging.getLogger(__name__)

__all__ = [
    "install",
    "uninstall",
    "is_installed",
    "maybe_install_from_env",
    "target_device",
    "resolve_fake_device",
    "run_scripted_loop",
    "run_pretrain",
    "registered_specs",
    "FakeEvent",
    "record_fake_event",
    "begin_run",
    "current_run_id",
    "verify_against_probing",
    "VerifyReport",
]

_INSTALLED = False


def registered_specs() -> dict[str, registry.FakeSpec]:
    return registry.registered()


def is_installed() -> bool:
    return _INSTALLED


def _env_force() -> bool:
    raw = os.environ.get("PROBING_FAKES_FORCE", "").strip().lower()
    return raw in {"1", "true", "on", "yes"}


def install(
    *,
    device: Optional[str] = None,
    specs: Iterable[str] | None = None,
    remap_device: bool = True,
    force: bool | None = None,
) -> None:
    """Enable fake specs and (by default) remap CUDA onto ``device``.

    ``force=True`` (or ``PROBING_FAKES_FORCE=1``) shadows real packages for the
    enabled prefixes — required when megatron-core is installed but broken on
    macOS (missing triton/CUDA).
    """
    global _INSTALLED
    from .journal import begin_run, record_fake_event

    use_force = _env_force() if force is None else bool(force)
    registry.enable(specs)
    set_force(use_force)
    if use_force:
        prefixes: list[str] = []
        for name in registry.enabled_names():
            prefixes.extend(registry.registered()[name].prefixes)
        removed = purge_prefixed_modules(prefixes)
        if removed:
            logger.info(
                "probing.fakes force-purged %d modules: %s",
                len(removed),
                ", ".join(sorted(removed)[:8]) + ("…" if len(removed) > 8 else ""),
            )
    install_finder()
    if remap_device:
        install_device_remap(device)
    install_torch_dist_hooks()
    run_id = begin_run()
    record_fake_event(
        "install",
        "probing.fakes",
        device=target_device() if is_device_remap_installed() else "",
        attrs={
            "specs": sorted(registry.enabled_names()),
            "force": force_enabled(),
            "run_id": run_id,
        },
    )
    _INSTALLED = True
    logger.info(
        "probing.fakes installed (run=%s specs=%s, device=%s, force=%s)",
        run_id,
        sorted(registry.enabled_names()),
        target_device() if is_device_remap_installed() else "unchanged",
        force_enabled(),
    )


def uninstall() -> None:
    """Remove the finder, device remap, and modules injected by fakes."""
    global _INSTALLED
    try:
        from .journal import record_fake_event

        record_fake_event("uninstall", "probing.fakes")
    except Exception:
        pass
    uninstall_torch_dist_hooks()
    uninstall_finder()
    clear_injected_modules()
    uninstall_device_remap()
    registry.disable_all()
    _INSTALLED = False
    logger.info("probing.fakes uninstalled")


def maybe_install_from_env() -> bool:
    """Install when ``PROBING_FAKES`` is set to an enabling value.

    Returns True if install ran.
    """
    names = registry.parse_fakes_env(os.environ.get("PROBING_FAKES"))
    if names is None:
        return False
    install(specs=names)
    return True


def run_scripted_loop(**kwargs):
    from .loop import run_scripted_loop as _run

    return _run(**kwargs)


def run_pretrain(**kwargs):
    from .pretrain import run_pretrain as _run

    return _run(**kwargs)


def __getattr__(name: str):
    if name == "FakeEvent":
        from .journal import FakeEvent

        return FakeEvent
    if name == "record_fake_event":
        from .journal import record_fake_event

        return record_fake_event
    if name == "begin_run":
        from .journal import begin_run

        return begin_run
    if name == "current_run_id":
        from .journal import current_run_id

        return current_run_id
    if name == "verify_against_probing":
        from .verify import verify_against_probing

        return verify_against_probing
    if name == "VerifyReport":
        from .verify import VerifyReport

        return VerifyReport
    raise AttributeError(f"module {__name__!r} has no attribute {name!r}")

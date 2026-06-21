"""Backward-compatible import path; see ``probing.tracing.hooks``."""

from probing.tracing.hooks import (  # noqa: F401
    PhaseTracker,
    _REGISTRY,
    attach_training_phases,
    detach_training_phases,
    maybe_auto_attach,
)

__all__ = [
    "PhaseTracker",
    "_REGISTRY",
    "attach_training_phases",
    "detach_training_phases",
    "maybe_auto_attach",
]

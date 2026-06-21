"""Tracing primitives for probing."""

from __future__ import annotations

from probing.tracing._bindings import Span, current_span
from probing.tracing.backends import (
    bind_table,
    configure as configure_backends,
    list_backends,
    register as register_backend,
    reset as reset_backends,
)
from probing.tracing.coordinates import row_fields, span_attrs, step, step_fields
from probing.tracing.hooks import (
    attach_training_phases,
    detach_training_phases,
    owns_training_phases,
)
from probing.tracing.phases import (
    BACKWARD,
    FORWARD,
    IDLE,
    OPTIMIZER,
    SOURCE_MANUAL,
    SOURCE_PHASE_HOOK,
    SOURCE_TORCH_PROBE,
    phase,
    reset_phase,
)
from probing.tracing.span import event, record_span, span
from probing.tracing.table import SPANS_SQL, TraceEvent

bind_table(TraceEvent)

__all__ = [
    "span",
    "event",
    "record_span",
    "current_span",
    "step",
    "step_fields",
    "row_fields",
    "span_attrs",
    "phase",
    "reset_phase",
    "attach_training_phases",
    "detach_training_phases",
    "owns_training_phases",
    "FORWARD",
    "BACKWARD",
    "OPTIMIZER",
    "IDLE",
    "SOURCE_MANUAL",
    "SOURCE_PHASE_HOOK",
    "SOURCE_TORCH_PROBE",
    "register_backend",
    "configure_backends",
    "list_backends",
    "reset_backends",
    "TraceEvent",
    "SPANS_SQL",
]

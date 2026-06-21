"""Rust tracing bindings (internal)."""

from __future__ import annotations

from probing import _core

try:
    Span = _core.Span
    current_span = _core.current_span
    active_span_for_events = _core.active_span_for_events
    active_span_by_phase = _core.active_span_by_phase
    active_training_phase = _core.active_training_phase
    step_snapshot = _core.py_step_snapshot
    sync_micro_step = _core.py_sync_micro_step
    advance_micro_step = _core.py_advance_micro_step
    set_micro_batches = _core.py_set_micro_batches
    current_micro_step = _core.py_current_micro_step
except AttributeError:
    Span = None

    def current_span():
        return None

    def active_span_for_events():
        return None

    def active_span_by_phase(_phase: str):
        return None

    def active_training_phase():
        return None

    def step_snapshot():
        return None

    def sync_micro_step(_step: int):
        return None

    def advance_micro_step():
        return None

    def set_micro_batches(_micro_batches: int):
        return None

    def current_micro_step() -> int:
        return 0

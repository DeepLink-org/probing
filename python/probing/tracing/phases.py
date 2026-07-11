"""Training phase vocabulary, inference, and runtime coordination."""

from __future__ import annotations

import time
from typing import Literal, Optional

FORWARD = "forward"
BACKWARD = "backward"
OPTIMIZER = "optimizer"
IDLE = "idle"

TrainingPhase = Literal["forward", "backward", "optimizer"]

ALL = frozenset({FORWARD, BACKWARD, OPTIMIZER})

# Span name → phase when ``phase`` is omitted.
_NAME_PHASE: dict[str, str] = {
    "forward": FORWARD,
    "backward": BACKWARD,
    "step": OPTIMIZER,
    "optimizer": OPTIMIZER,
}

# Span names that must not infer a training phase.
_NON_PHASE_NAMES = frozenset(
    {"train.step", "model.init", "data.load", "checkpoint.save"}
)

# Who emitted a span (for composability / dedup).
SOURCE_MANUAL = "manual"
SOURCE_PHASE_HOOK = "phase_hook"
SOURCE_TORCH_PROBE = "torch_probe"

_hook_spans: dict[str, object] = {}
_iteration_start_ns: Optional[int] = None


def infer(name: str) -> Optional[str]:
    if not name or name in _NON_PHASE_NAMES:
        return None
    if name in _NAME_PHASE:
        return _NAME_PHASE[name]
    base = name.rsplit(".", 1)[-1]
    return _NAME_PHASE.get(base)


def infer_from_stage(stage: str) -> Optional[str]:
    """Map TorchProbe hook stage label to training phase."""
    lowered = stage.lower()
    for token in ("optimizer", "backward", "forward", "step"):
        if token in lowered:
            mapped = infer(token)
            if mapped is not None:
                return mapped
    return None


def resolve(name: str, phase: Optional[str]) -> Optional[str]:
    if phase is not None:
        if phase not in ALL:
            raise ValueError(
                f"invalid training phase {phase!r}; use FORWARD, BACKWARD, or OPTIMIZER"
            )
        return phase
    return infer(name)


def resolve_span(
    name: Optional[str] = None,
    phase: Optional[str] = None,
) -> tuple[str, Optional[str]]:
    """Return ``(span_name, training_phase)``. Requires at least one of *name* or *phase*.

    When *phase* is given and *name* is omitted, ``span_name == phase`` (canonical form).
    """
    if phase is not None:
        resolved = resolve(name or phase, phase)
        display = name if name is not None else resolved
        assert display is not None
        return display, resolved
    if name is not None:
        return name, resolve(name, None)
    raise TypeError("span() requires name and/or phase")


def is_training_phase(value: Optional[str]) -> bool:
    return value in ALL


def phase() -> str:
    """Current training phase from the innermost active phase span, else ``idle``."""
    from probing.tracing._bindings import active_training_phase

    active = active_training_phase()
    return active if active is not None else IDLE


def reset_phase() -> None:
    """Reset coordinator state (tests). Closes any hook spans left open."""
    global _iteration_start_ns
    while _hook_spans:
        _, handle = _hook_spans.popitem()
        try:
            handle.__exit__(None, None, None)
        except Exception:
            pass
    _iteration_start_ns = None


def hook_enter(span_phase: str) -> None:
    global _iteration_start_ns
    if span_phase == FORWARD and _iteration_start_ns is None:
        _iteration_start_ns = time.time_ns()
    if _active_phase_span(span_phase) or span_phase in _hook_spans:
        return
    import probing

    handle = probing.span(phase=span_phase, source=SOURCE_PHASE_HOOK)
    handle.__enter__()
    _hook_spans[span_phase] = handle


def hook_exit(span_phase: str) -> None:
    if span_phase == OPTIMIZER:
        _record_train_step()
    handle = _hook_spans.pop(span_phase, None)
    if handle is not None:
        handle.__exit__(None, None, None)


def _record_train_step() -> None:
    """Record one ``train.step`` closed span for the current logical iteration."""
    global _iteration_start_ns
    if _iteration_start_ns is None:
        return
    import probing

    from probing.tracing.coordinates import step, step_fields

    snap = step.snapshot()
    mb = max(int(snap.micro_batches), 1)
    micro = int(snap.micro_step)
    attrs = {
        **step_fields(snap),
        "accum_index": micro % mb,
        "logical_step_pending": micro // mb,
    }
    duration_ns = int(time.time_ns()) - _iteration_start_ns
    _iteration_start_ns = None
    probing.record_span(
        "train.step",
        duration_ns=duration_ns,
        attrs=attrs,
        source=SOURCE_PHASE_HOOK,
    )


def _active_phase_span(span_phase: str) -> bool:
    from probing.tracing._bindings import active_span_by_phase

    return active_span_by_phase(span_phase) is not None

"""RL-oriented tracing helpers.

This module keeps the public API small and framework-neutral.  It records RL
semantics as regular probing span attributes so existing Ray/process tracing,
queries, and Chrome tracing export continue to work.
"""

from __future__ import annotations

import contextvars
from contextlib import asynccontextmanager, contextmanager
from typing import Any, Callable, Optional

from probing.tracing import event as probing_event
from probing.tracing import span as probing_span

STANDARD_ATTRS = (
    "run_id",
    "framework",
    "algorithm",
    "rollout_id",
    "step_id",
    "sample_id",
    "trajectory_id",
    "group_id",
    "attempt",
    "turn_id",
    "env_step_id",
    "phase",
    "actor_role",
    "batch_id",
)

_current_context = contextvars.ContextVar("probing_rl_context", default={})


def current_context() -> dict[str, Any]:
    """Return the active RL trace context for this Python context."""

    return dict(_current_context.get() or {})


def normalize_attrs(**attrs: Any) -> dict[str, Any]:
    """Normalize common aliases and drop ``None`` values."""

    normalized = {key: value for key, value in attrs.items() if value is not None}
    if "trajectory_id" not in normalized and "sample_id" in normalized:
        normalized["trajectory_id"] = normalized["sample_id"]
    if "sample_id" not in normalized and "trajectory_id" in normalized:
        normalized["sample_id"] = normalized["trajectory_id"]
    return normalized


def merge_context(**attrs: Any) -> dict[str, Any]:
    """Merge active RL context with explicit attributes.

    Explicit attributes win over context values.  The result is suitable for
    passing directly to ``probing.span``.
    """

    merged = current_context()
    merged.update(normalize_attrs(**attrs))
    return {key: value for key, value in merged.items() if value is not None}


@contextmanager
def context(**attrs: Any):
    """Temporarily attach RL identity attributes to nested spans/events."""

    merged = merge_context(**attrs)
    token = _current_context.set(merged)
    try:
        yield merged
    finally:
        _current_context.reset(token)


@contextmanager
def span(name: str, *, phase: Optional[str] = None, kind: str = "rl.phase", **attrs: Any):
    """Record an RL span with standard rollout/sample attributes."""

    merged = merge_context(**attrs)
    span_phase = phase if phase is not None else kind
    if phase is not None:
        merged["phase"] = phase
    else:
        merged.setdefault("phase", name)
    with probing_span(name, phase=span_phase, **merged) as active_span:
        yield active_span


@asynccontextmanager
async def async_span(
    name: str,
    *,
    phase: Optional[str] = None,
    kind: str = "rl.phase",
    **attrs: Any,
):
    """Async variant of :func:`span` for asyncio-heavy rollout code."""

    with span(name, phase=phase, kind=kind, **attrs) as active_span:
        yield active_span


def event(name: str, *, phase: Optional[str] = None, **attrs: Any) -> None:
    """Record an instant RL event on the current probing span."""

    merged = merge_context(**attrs)
    if phase is not None:
        merged["phase"] = phase
    probing_event(name, attributes=[merged] if merged else None)


def bind(**attrs: Any) -> dict[str, Any]:
    """Build a normalized context carrier that can be sent across Ray calls."""

    return normalize_attrs(**attrs)


def export_context(**attrs: Any) -> dict[str, Any]:
    """Export the active RL context plus optional overrides."""

    return merge_context(**attrs)


def import_context(carrier: Optional[dict[str, Any]] = None, **attrs: Any) -> dict[str, Any]:
    """Normalize a received carrier before using it with :func:`context`."""

    merged = dict(carrier or {})
    merged.update(attrs)
    return normalize_attrs(**merged)


def decorator(
    name: str,
    *,
    target_getter: Optional[Callable[..., dict[str, Any]]] = None,
    attrs_getter: Optional[Callable[..., dict[str, Any]]] = None,
    phase: Optional[str] = None,
    kind: str = "rl.phase",
):
    """Decorate sync or async functions with an RL span.

    ``target_getter`` is intended for framework adapters that can extract a
    sample/trajectory context from function arguments without hardcoding a
    specific RL framework into probing itself.
    """

    import functools
    import inspect

    def wrap(fn):
        def build_attrs(args, kwargs):
            merged = {}
            if target_getter is not None:
                target_attrs = target_getter(*args, **kwargs)
                if target_attrs:
                    merged.update(target_attrs)
            if attrs_getter is not None:
                extra_attrs = attrs_getter(*args, **kwargs)
                if extra_attrs:
                    merged.update(extra_attrs)
            return merged

        if inspect.iscoroutinefunction(fn):

            @functools.wraps(fn)
            async def async_wrapper(*args, **kwargs):
                with span(name, phase=phase, kind=kind, **build_attrs(args, kwargs)):
                    return await fn(*args, **kwargs)

            return async_wrapper

        @functools.wraps(fn)
        def sync_wrapper(*args, **kwargs):
            with span(name, phase=phase, kind=kind, **build_attrs(args, kwargs)):
                return fn(*args, **kwargs)

        return sync_wrapper

    return wrap


__all__ = [
    "STANDARD_ATTRS",
    "async_span",
    "bind",
    "context",
    "current_context",
    "decorator",
    "event",
    "export_context",
    "import_context",
    "merge_context",
    "normalize_attrs",
    "span",
]

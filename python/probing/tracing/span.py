"""Span lifecycle: open, event, record."""

from __future__ import annotations

import functools
import inspect
import json
import os
import time
from typing import Callable, Optional

from probing.tracing._bindings import (
    Span,
    active_span_by_phase,
    active_span_for_events,
    current_span,
)
from probing.tracing.coordinates import span_attrs, step
from probing.tracing.phases import OPTIMIZER, resolve_span
from probing.tracing.table import TraceEvent

_LOCATION_ENV = frozenset({"1", "true", "yes", "on"})


class _RecordedSpan:
    """Context manager: span stack + backend persistence."""

    def __init__(
        self,
        name: str,
        phase: Optional[str] = None,
        location: Optional[str] = None,
        attrs: Optional[dict] = None,
        *,
        source: str = "manual",
        auto_location: bool = False,
    ):
        self.name = name
        self.phase = phase
        self.location = location
        self.attrs = dict(attrs or {})
        self.source = source
        self._auto_location = auto_location
        self._span = None
        self._reentrant = False
        self._owns_step_advance = False

    def __enter__(self):
        if self.phase == OPTIMIZER:
            existing = active_span_by_phase(OPTIMIZER)
            if existing is not None:
                self._span = existing
                self._reentrant = True
                return existing

        loc = self.location
        if loc is None and self._auto_location:
            loc = _caller_location()
        merged = span_attrs(self.attrs, source=self.source)
        self._span = _create_span(self.name, self.phase, loc, merged)
        self._span.__enter__()
        _persist_span_start(self._span, merged)
        if self.phase == OPTIMIZER:
            self._owns_step_advance = True
        return self._span

    def __exit__(self, exc_type, exc_val, exc_tb):
        if self._span is None or self._reentrant:
            return False
        result = self._span.__exit__(exc_type, exc_val, exc_tb)
        _persist_span_end(self._span)
        if self._owns_step_advance:
            step()
        return result


def _create_span(name: str, phase: Optional[str], location: Optional[str], attrs: dict):
    parent = current_span()
    if parent:
        span_obj = Span.new_child(parent, name, phase=phase, location=location)
    else:
        span_obj = Span(name, phase=phase, location=location)
    if attrs and hasattr(span_obj, "_set_initial_attrs"):
        try:
            span_obj._set_initial_attrs(dict(attrs))
        except Exception as e:
            import warnings

            warnings.warn(f"Failed to set initial attributes: {e}")
    return span_obj


def _caller_location() -> Optional[str]:
    """Walk ``inspect.stack()`` for the first frame outside ``probing/tracing``."""
    try:
        for frame_info in inspect.stack()[2:]:
            path = frame_info.filename.replace("\\", "/")
            if "probing/tracing" in path:
                continue
            return f"{frame_info.filename}:{frame_info.function}:{frame_info.lineno}"
    except Exception:
        pass
    return None


def _location_enabled() -> bool:
    return os.environ.get("PROBING_SPAN_LOCATION", "").lower() in _LOCATION_ENV


def _span_options(
    kwargs: dict,
) -> tuple[str, Optional[str], str, Optional[str], dict, bool]:
    phase = kwargs.pop("phase", None)
    source = kwargs.pop("source", "manual")
    location = kwargs.pop("location", None)
    auto_location = location is None and _location_enabled()
    return phase, source, location, kwargs, auto_location


def _make_handle(
    name: str,
    phase: Optional[str],
    location: Optional[str],
    attrs: dict,
    source: str,
    auto_location: bool,
):
    class SpanHandle:
        def __init__(self):
            self.name = name
            self.phase = phase
            self.location = location
            self.source = source
            self.attrs = attrs
            self._auto_location = auto_location
            self._inner = None

        def __call__(self, func: Callable) -> Callable:
            @functools.wraps(func)
            def wrapper(*wargs, **wkwargs):
                with _RecordedSpan(
                    self.name,
                    phase=self.phase,
                    location=self.location,
                    attrs=self.attrs,
                    source=self.source,
                    auto_location=self._auto_location,
                ):
                    return func(*wargs, **wkwargs)

            return wrapper

        def __enter__(self):
            self._inner = _RecordedSpan(
                self.name,
                phase=self.phase,
                location=self.location,
                attrs=self.attrs,
                source=self.source,
                auto_location=self._auto_location,
            )
            return self._inner.__enter__()

        def __exit__(self, *exc):
            if self._inner:
                return self._inner.__exit__(*exc)
            return False

        def __getattr__(self, attr):
            if self._inner is not None:
                return getattr(self._inner, attr)
            raise AttributeError(attr)

    return SpanHandle()


def span(*args, **kwargs):
    """Open a span (context manager, decorator, or manual enter/exit).

    Reserved kwargs: ``phase``, ``source``, ``location``. Training phases are
    ``FORWARD``, ``BACKWARD``, ``OPTIMIZER`` (see ``probing.tracing.phases``).

    When ``phase`` is set and ``name`` is omitted, ``name`` defaults to ``phase``.
    When only ``name`` is given, phase is inferred (e.g. ``"forward"`` → ``FORWARD``).

    Auto ``location`` via ``inspect.stack()`` is off by default; set
    ``PROBING_SPAN_LOCATION=1`` or pass ``location=...`` explicitly.
    """
    phase_kw, source, location, attrs, auto_location = _span_options(dict(kwargs))

    if len(args) == 1 and isinstance(args[0], str):
        name_kw = args[0]
        name, phase = resolve_span(name_kw, phase_kw)
        return _make_handle(name, phase, location, attrs, source, auto_location)

    if len(args) == 0:
        if phase_kw is not None:
            name, phase = resolve_span(None, phase_kw)
            return _make_handle(name, phase, location, attrs, source, auto_location)
        if not attrs:

            def decorator(func: Callable) -> Callable:
                resolved_name, resolved_phase = resolve_span(func.__name__, None)
                return _make_handle(
                    resolved_name,
                    resolved_phase,
                    location,
                    {},
                    source,
                    auto_location,
                )(func)

            return decorator
        raise TypeError("span() requires name and/or phase")

    if len(args) == 1 and callable(args[0]):
        func = args[0]
        resolved_name, resolved_phase = resolve_span(func.__name__, phase_kw)

        @functools.wraps(func)
        def wrapper(*wargs, **wkwargs):
            with _RecordedSpan(
                resolved_name,
                phase=resolved_phase,
                location=location,
                attrs=attrs,
                source=source,
                auto_location=auto_location,
            ):
                return func(*wargs, **wkwargs)

        return wrapper

    if len(args) == 1:
        raise TypeError(
            f"span() first argument must be str or callable, got {type(args[0]).__name__}"
        )
    if len(args) > 1:
        raise TypeError("span() takes at most one positional argument")

    raise TypeError("span() requires at least one argument")


def event(name: str, *, attributes: Optional[list] = None):
    """Add a point event on the active span."""
    current = active_span_for_events() or current_span()
    if current is None or getattr(current, "is_ended", False):
        raise RuntimeError("No active span in current context. Cannot add event.")
    current.add_event(name, attributes=attributes)


def record_span(
    name: str,
    *,
    phase: Optional[str] = None,
    duration_ns: int,
    attrs: Optional[dict] = None,
    source: str = "manual",
) -> None:
    """Record a completed span without entering the span stack (hot path)."""
    if duration_ns < 0:
        duration_ns = 0

    TraceEvent.init_table()
    merged = span_attrs(dict(attrs or {}), source=source)
    end_ns = int(time.time_ns())
    start_ns = end_ns - duration_ns
    resolved_name, resolved_phase = resolve_span(name, phase)

    parent = current_span()
    if parent:
        span_obj = Span.new_child(
            parent, resolved_name, phase=resolved_phase, location=""
        )
    else:
        span_obj = Span(resolved_name, phase=resolved_phase, location="")

    from probing.tracing.backends import get_recorder

    get_recorder().record_closed_span(
        span_obj,
        name=resolved_name,
        phase=resolved_phase or "",
        start_ns=start_ns,
        end_ns=end_ns,
        attributes_json=json.dumps(merged) if merged else "",
    )


def _persist_span_start(span: Span, attrs: dict) -> None:
    from probing.tracing.backends import get_recorder

    get_recorder().record_span_start(span, attrs)


def _persist_span_end(span: Span) -> None:
    from probing.tracing.backends import get_recorder

    get_recorder().record_span_end(span)


def _persist_event(
    span: Span, event_name: str, event_attributes: Optional[list] = None
) -> None:
    from probing.tracing.backends import get_recorder

    get_recorder().record_event(span, event_name, event_attributes)


if Span:
    _rust_add_event = Span.add_event

    def _add_event_persist(self, name, attributes=None):
        _rust_add_event(self, name, attributes=attributes)
        _persist_event(self, name, attributes)

    Span.add_event = _add_event_persist

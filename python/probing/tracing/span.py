"""Span lifecycle: open, event, record."""

from __future__ import annotations

import functools
import inspect
import json
import os
import time
import warnings
from dataclasses import dataclass
from typing import Callable, Optional

from probing.tracing._bindings import (
    Span,
    active_span_by_phase,
    active_span_for_events,
    current_span,
)
from probing.tracing.coordinates import span_attrs, step
from probing.tracing.phases import OPTIMIZER, resolve_span

_LOCATION_ENV = frozenset({"1", "true", "yes", "on"})

# Rust Span cannot hold arbitrary Python attrs; track deferred persistence by id.
_DEFERRED: dict[int, "_DeferredState"] = {}


@dataclass
class _DeferredState:
    merged: dict
    start_persisted: bool = False


def _recorder():
    from probing.tracing.backends import get_recorder

    return get_recorder()


def _persistence_enabled() -> bool:
    from probing.tracing.backends import persistence_enabled

    return persistence_enabled()


def _spawn_span(
    name: str, phase: Optional[str], *, location: Optional[str] = None
) -> Span:
    parent = current_span()
    if parent:
        return Span.new_child(parent, name, phase=phase, location=location)
    return Span(name, phase=phase, location=location)


def _attach_attrs(span_obj: Span, attrs: dict) -> None:
    if not attrs or not hasattr(span_obj, "_set_initial_attrs"):
        return
    try:
        span_obj._set_initial_attrs(dict(attrs))
    except Exception as exc:
        warnings.warn(f"Failed to set initial attributes: {exc}")


class _RecordedSpan:
    """Context manager: span stack + backend persistence on close."""

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
        self._span: Optional[Span] = None
        self._reentrant = False
        self._owns_step_advance = False
        self._persist = False
        self._merged: dict = {}

    def __enter__(self) -> Span:
        if self.phase == OPTIMIZER:
            existing = active_span_by_phase(OPTIMIZER)
            if existing is not None and not getattr(existing, "is_ended", False):
                self._span = existing
                self._reentrant = True
                return existing

        location = self.location
        if location is None and self._auto_location:
            location = _caller_location()

        self._persist = _persistence_enabled()
        self._merged = (
            span_attrs(self.attrs, source=self.source) if self._persist else {}
        )

        span_obj = _spawn_span(self.name, self.phase, location=location)
        _attach_attrs(span_obj, self._merged)
        span_obj.__enter__()
        self._span = span_obj

        if self._persist:
            _DEFERRED[int(span_obj.span_id)] = _DeferredState(merged=self._merged)
        if self.phase == OPTIMIZER:
            self._owns_step_advance = True
        return span_obj

    def __exit__(self, exc_type, exc_val, exc_tb) -> bool:
        if self._span is None or self._reentrant:
            return False

        result = self._span.__exit__(exc_type, exc_val, exc_tb)
        state = _DEFERRED.pop(int(self._span.span_id), None)
        if state is not None:
            _persist_on_close(self._span, state)
        if self._owns_step_advance:
            step()
        return result


class _SpanHandle:
    """Deferred ``probing.span()`` entry (context manager or decorator)."""

    def __init__(
        self,
        name: str,
        phase: Optional[str],
        location: Optional[str],
        attrs: dict,
        source: str,
        auto_location: bool,
    ) -> None:
        self._name = name
        self._phase = phase
        self._location = location
        self._attrs = attrs
        self._source = source
        self._auto_location = auto_location
        self._inner: Optional[_RecordedSpan] = None

    def _make_cm(self) -> _RecordedSpan:
        return _RecordedSpan(
            self._name,
            phase=self._phase,
            location=self._location,
            attrs=self._attrs,
            source=self._source,
            auto_location=self._auto_location,
        )

    def __call__(self, func: Callable) -> Callable:
        @functools.wraps(func)
        def wrapper(*args, **kwargs):
            with self._make_cm():
                return func(*args, **kwargs)

        return wrapper

    def __enter__(self) -> Span:
        self._inner = self._make_cm()
        return self._inner.__enter__()

    def __exit__(self, *exc) -> bool:
        if self._inner is None:
            return False
        return self._inner.__exit__(*exc)

    def __getattr__(self, attr: str):
        if self._inner is not None:
            return getattr(self._inner, attr)
        raise AttributeError(attr)


def _caller_location() -> Optional[str]:
    """First stack frame outside ``probing/tracing``."""
    try:
        for frame in inspect.stack()[2:]:
            path = frame.filename.replace("\\", "/")
            if "probing/tracing" in path:
                continue
            return f"{frame.filename}:{frame.function}:{frame.lineno}"
    except Exception:
        pass
    return None


def _location_enabled() -> bool:
    return os.environ.get("PROBING_SPAN_LOCATION", "").lower() in _LOCATION_ENV


def _parse_span_kwargs(
    kwargs: dict,
) -> tuple[Optional[str], str, Optional[str], dict, bool]:
    phase = kwargs.pop("phase", None)
    source = kwargs.pop("source", "manual")
    location = kwargs.pop("location", None)
    auto_location = location is None and _location_enabled()
    return phase, source, location, kwargs, auto_location


def _handle(
    name: str,
    phase: Optional[str],
    location: Optional[str],
    attrs: dict,
    source: str,
    auto_location: bool,
) -> _SpanHandle:
    return _SpanHandle(name, phase, location, attrs, source, auto_location)


def span(*args, **kwargs):
    """Open a span (context manager, decorator, or manual enter/exit).

    Reserved kwargs: ``phase``, ``source``, ``location``. Training phases are
    ``FORWARD``, ``BACKWARD``, ``OPTIMIZER`` (see ``probing.tracing.phases``).

    When ``phase`` is set and ``name`` is omitted, ``name`` defaults to ``phase``.
    When only ``name`` is given, phase is inferred (e.g. ``"forward"`` → ``FORWARD``).

    Auto ``location`` via ``inspect.stack()`` is off by default; set
    ``PROBING_SPAN_LOCATION=1`` or pass ``location=...`` explicitly.
    """
    phase_kw, source, location, attrs, auto_location = _parse_span_kwargs(dict(kwargs))

    if len(args) > 1:
        raise TypeError("span() takes at most one positional argument")

    if len(args) == 1 and callable(args[0]):
        name, phase = resolve_span(args[0].__name__, phase_kw)
        return _handle(name, phase, location, attrs, source, auto_location)(args[0])

    if len(args) == 1:
        if not isinstance(args[0], str):
            raise TypeError(
                f"span() first argument must be str or callable, got {type(args[0]).__name__}"
            )
        name, phase = resolve_span(args[0], phase_kw)
        return _handle(name, phase, location, attrs, source, auto_location)

    if phase_kw is not None:
        name, phase = resolve_span(None, phase_kw)
        return _handle(name, phase, location, attrs, source, auto_location)

    if attrs:
        raise TypeError("span() requires name and/or phase")

    def decorator(func: Callable) -> Callable:
        name, phase = resolve_span(func.__name__, None)
        return _handle(name, phase, location, {}, source, auto_location)(func)

    return decorator


def event(name: str, *, attributes: Optional[list] = None) -> None:
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
    recorder = _recorder()
    if not recorder.enabled:
        return

    duration_ns = max(duration_ns, 0)
    merged = span_attrs(dict(attrs or {}), source=source)
    end_ns = int(time.time_ns())
    start_ns = end_ns - duration_ns
    resolved_name, resolved_phase = resolve_span(name, phase)

    span_obj = _spawn_span(resolved_name, resolved_phase, location="")
    recorder.record_closed_span(
        span_obj,
        name=resolved_name,
        phase=resolved_phase or "",
        start_ns=start_ns,
        end_ns=end_ns,
        attributes_json=json.dumps(merged) if merged else "",
    )


def _persist_on_close(span: Span, state: _DeferredState) -> None:
    if state.start_persisted:
        _persist_span_end(span)
    else:
        _persist_closed(span, state.merged)


def _persist_span_start(span: Span, attrs: dict) -> None:
    recorder = _recorder()
    if not recorder.enabled:
        return
    recorder.record_span_start(span, attrs)
    state = _DEFERRED.get(int(span.span_id))
    if state is not None:
        state.start_persisted = True


def _persist_span_end(span: Span) -> None:
    recorder = _recorder()
    if recorder.enabled:
        recorder.record_span_end(span)


def _persist_closed(span: Span, attrs: dict) -> None:
    recorder = _recorder()
    if not recorder.enabled:
        return
    phase = getattr(span, "phase", None) or ""
    recorder.record_closed_span(
        span,
        name=str(span.name),
        phase=str(phase),
        start_ns=int(span.start_timestamp),
        end_ns=int(span.end_timestamp or time.time_ns()),
        attributes_json=json.dumps(attrs) if attrs else "",
    )


def _persist_event(
    span: Span, event_name: str, event_attributes: Optional[list] = None
) -> None:
    recorder = _recorder()
    if not recorder.enabled:
        return
    state = _DEFERRED.get(int(span.span_id))
    if state is not None and not state.start_persisted:
        _persist_span_start(span, state.merged)
    recorder.record_event(span, event_name, event_attributes)


if Span:
    _rust_add_event = Span.add_event

    def _add_event_persist(self, name, attributes=None):
        _rust_add_event(self, name, attributes=attributes)
        _persist_event(self, name, attributes)

    Span.add_event = _add_event_persist

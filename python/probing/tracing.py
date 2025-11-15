"""Tracing facade (Python side).

Provides a thin, explicit wrapper around the Rust implementation for creating spans
via a context manager or decorator, attaching immutable attributes at creation time,
and recording span lifecycle plus custom events into a single table.

Notes
-----
* Attributes are fixed at span creation (no mutation API exposed).
* `TraceEvent` stores start/end/event rows; missing values use simple sentinels
  (parent_id = -1, text fields = empty string) to avoid `None` persistence issues.
* The public surface stays minimal: `span`, `Span.with_`, `Span.decorator`, `add_event`,
  and the `TraceEvent` dataclass table.

Examples
--------
Context manager::

    import probing
    with probing.span("load_data", dataset="mnist") as s:
        probing.event("read")
        # do work

Decorator::

    import probing
    @probing.span("predict")
    def predict(x):
        return model(x)

Implicit name decorator::

    import probing
    @probing.span
    def compute():
        return 42
"""

import functools
import inspect
from dataclasses import dataclass
from typing import Any, Callable, Optional

# Import from the internal Rust module
from probing._tracing import Span
from probing._tracing import _span_raw as span_raw
from probing._tracing import current_span
from probing.core.table import table


def _get_location() -> Optional[str]:
    """Get the current call location from the stack.

    Returns
    -------
    Optional[str]
        Location string in format "filename:function:lineno" or None if unavailable.
    """
    try:
        # Get the frame that called span() (skip this function and span() itself)
        stack = inspect.stack()
        # Find the first frame that's not in this module
        for frame_info in stack[2:]:  # Skip _get_location and span()
            frame = frame_info.frame
            filename = frame_info.filename
            function = frame_info.function
            lineno = frame_info.lineno

            # Skip frames from this module
            if "probing/tracing.py" in filename or "probing\\tracing.py" in filename:
                continue

            # Format: "filename:function:lineno"
            return f"{filename}:{function}:{lineno}"
    except Exception:
        pass
    return None


@table
@dataclass
class TraceEvent:
    """Row model for trace records.

    Each saved instance is one of: span_start, span_end, event.

    Parameters
    ----------
    record_type : str
        One of ``'span_start'``, ``'span_end'`` or ``'event'``.
    trace_id : int
        Trace identifier (shared by related spans).
    span_id : int
        Unique span identifier.
    name : str
        Span or event name.
    time : int
        Nanoseconds since epoch.
    parent_id : int, default -1
        Parent span id, -1 if root.
    kind : str, default ""
        Optional span kind label.
    location : str, default ""
        Code location automatically captured from call stack.
    attributes : str, default ""
        JSON string of span attributes (only in span rows).
    event_attributes : str, default ""
        JSON string of event attributes (only in event rows).
    """

    # Required fields
    record_type: str
    trace_id: int
    span_id: int
    name: str
    time: int
    thread_id: int = 0

    # Optional fields
    parent_id: Optional[int] = -1
    kind: Optional[str] = ""
    location: Optional[str] = ""
    attributes: Optional[str] = ""
    event_attributes: Optional[str] = ""


def span(*args, **kwargs):
    """Factory for span usage as context manager or decorator.

    Scenarios
    ---------
    1. Context manager::

        with span("work", user="alice") as s:
            ...

    2. Decorator with explicit name::

        @span("inference")
        def run(x): ...

    3. Decorator with implicit function name::

        @span
        def train(): ...

    Parameters
    ----------
    *args
        Either empty (implicit decorator), a single callable, or a single string name.
    **kwargs
        Attributes to attach plus optional ``kind``.

    Note
    ----
    The ``location`` is automatically captured from the call stack using
    Python's ``inspect`` module. It is not passed as a parameter.

    Returns
    -------
    object
        A context manager / decorator hybrid or a pure decorator.
    """
    # Extract special parameters
    kind = kwargs.pop("kind", None)
    # Location is automatically captured, not passed as parameter
    location = _get_location()

    # Handle @span (without arguments) - no args and no kwargs
    if len(args) == 0 and not kwargs:

        def decorator(func: Callable) -> Callable:
            @functools.wraps(func)
            def wrapper(*wargs, **wkwargs):
                # Get location from the decorator's call site
                loc = _get_location()
                with span_raw(func.__name__, kind=kind, location=loc) as s:
                    return func(*wargs, **wkwargs)

            return wrapper

        return decorator

    # Handle @span(func) - first arg is a callable
    if len(args) == 1 and callable(args[0]):
        func = args[0]

        @functools.wraps(func)
        def wrapper(*wargs, **wkwargs):
            # Get location from the decorator's call site
            loc = _get_location()
            with span_raw(func.__name__, kind=kind, location=loc) as s:
                return func(*wargs, **wkwargs)

        return wrapper

    # Handle @span("name") or with span("name")
    if len(args) == 1 and isinstance(args[0], str):
        name = args[0]

        # Create a wrapper that supports both decorator and context manager usage
        class SpanWrapper:
            def __init__(
                self,
                name: str,
                kind: Optional[str],
                location: Optional[str],
                attrs: dict,
            ):
                self.name = name
                self.kind = kind
                self.location = location
                self.attrs = attrs
                self._span = None

            def __call__(self, func: Callable) -> Callable:
                """Enable decorator form when a name was provided.

                Parameters
                ----------
                func : Callable
                    Function to wrap.

                Returns
                -------
                Callable
                    Wrapped function executing inside a span.
                """

                @functools.wraps(func)
                def wrapper(*wargs, **wkwargs):
                    # Create span with attributes set during creation
                    # Get location from the decorator's call site
                    loc = _get_location()
                    if self.attrs:
                        # Use span() function which handles attributes during creation
                        with span(
                            self.name,
                            kind=self.kind,
                            **self.attrs,
                        ) as s:
                            return func(*wargs, **wkwargs)
                    else:
                        with span_raw(self.name, kind=self.kind, location=loc) as s:
                            return func(*wargs, **wkwargs)

                return wrapper

            def __enter__(self):
                """Enter span context.

                Returns
                -------
                Span
                    The underlying span instance.
                """
                # Get current span for parent relationship
                parent = current_span()
                # Get location from the context manager's call site
                loc = self.location or _get_location()

                if parent:
                    self._span = Span.new_child(
                        parent, self.name, kind=self.kind, location=loc
                    )
                else:
                    self._span = Span(self.name, kind=self.kind, location=loc)

                # Set initial attributes during creation (before __enter__)
                if self.attrs:
                    # Convert Python dict to dict that can be passed to _set_initial_attrs
                    attrs_dict = dict(self.attrs)
                    if hasattr(self._span, "_set_initial_attrs"):
                        try:
                            self._span._set_initial_attrs(attrs_dict)
                        except Exception as e:
                            # If setting attributes fails, log but continue
                            import warnings

                            warnings.warn(f"Failed to set initial attributes: {e}")

                # Enter the span context manager
                # This pushes the span to the thread-local stack
                # We need to call __enter__ to push it to the stack, but we can return the span directly
                entered = self._span.__enter__()
                # __enter__ returns PyRef<Span> which should be automatically converted
                # But to be safe, we return the span object directly

                # Record span start to table
                _record_span_start(self._span, self.attrs)

                return self._span

            def __exit__(self, *args):
                """Exit span context: finalize then record minimal end info."""
                if self._span:
                    result = self._span.__exit__(
                        *args
                    )  # finalize span first (sets end timestamp)
                    _record_span_end(self._span)  # then record minimal end row
                    return result
                return False

        return SpanWrapper(name, kind, location, kwargs)

    # Default: use as context manager with first arg as name
    if len(args) > 0:
        name = args[0]
        if not isinstance(name, str):
            raise TypeError("span() requires a string name as the first argument")

        # Get current span for parent relationship
        parent = current_span()
        # Get location from the call site
        loc = location or _get_location()

        if parent:
            span_obj = Span.new_child(parent, name, kind=kind, location=loc)
        else:
            span_obj = Span(name, kind=kind, location=loc)

        # Set initial attributes during creation
        if kwargs:
            attrs_dict = dict(kwargs)
            if hasattr(span_obj, "_set_initial_attrs"):
                span_obj._set_initial_attrs(attrs_dict)

        return span_obj

    raise TypeError("span() requires at least one argument")


def _record_span_start(span: Span, attrs: dict):
    """Persist span start.

    Parameters
    ----------
    span : Span
        Span object.
    attrs : dict
        Creation-time attributes.
    """
    import json

    # Convert attributes to JSON string
    attrs_json = None
    if attrs:
        attrs_json = json.dumps(attrs)
    # Sanitize None values to backend-friendly sentinels (tables reject Python None)
    parent_id = span.parent_id if span.parent_id is not None else -1
    kind = span.kind if span.kind is not None else ""
    location = (
        span.location if hasattr(span, "location") and span.location is not None else ""
    )
    attributes = attrs_json if attrs_json is not None else ""
    event = TraceEvent(
        record_type="span_start",
        trace_id=span.trace_id,
        span_id=span.span_id,
        name=span.name,
        time=span.start_timestamp,
        thread_id=getattr(span, "thread_id", 0),
        parent_id=parent_id,
        kind=kind,
        location=location,
        attributes=attributes,
        event_attributes="",  # not applicable
    )
    event.save()


def _record_span_end(span: Span):
    """Persist span end with minimal data (only end time + span id).

    Other fields are blanked to reduce duplication.
    """
    import time

    end_ts = span.end_timestamp or int(time.time_ns())
    event = TraceEvent(
        record_type="span_end",
        trace_id=0,  # intentionally zeroed
        span_id=span.span_id,
        name="",  # omit name
        time=end_ts,
        thread_id=getattr(span, "thread_id", 0),
        parent_id=-1,
        kind="",
        location="",
        attributes="",
        event_attributes="",
    )
    event.save()


def _record_event(span: Span, event_name: str, event_attributes: Optional[list] = None):
    """Persist an event row.

    Parameters
    ----------
    span : Span
        Active span.
    event_name : str
        Event name.
    event_attributes : list, optional
        List of dicts or (key, value) tuples.
    """
    import json
    import time

    # Get current timestamp (nanoseconds since epoch)
    timestamp = int(time.time_ns())

    # Convert event attributes to JSON string
    event_attrs_json = None
    if event_attributes:
        # Convert list of dicts/tuples to a single dict
        attrs_dict = {}
        for attr_item in event_attributes:
            if isinstance(attr_item, dict):
                attrs_dict.update(attr_item)
            elif isinstance(attr_item, (list, tuple)) and len(attr_item) == 2:
                attrs_dict[attr_item[0]] = attr_item[1]
        if attrs_dict:
            event_attrs_json = json.dumps(attrs_dict)

    parent_id = span.parent_id if span.parent_id is not None else -1
    kind = span.kind if span.kind is not None else ""
    location = (
        span.location if hasattr(span, "location") and span.location is not None else ""
    )
    attrs = ""  # span-level attributes not duplicated here
    event_attrs = event_attrs_json if event_attrs_json is not None else ""
    event = TraceEvent(
        record_type="event",
        trace_id=span.trace_id,
        span_id=span.span_id,
        name=event_name,
        time=timestamp,
        thread_id=getattr(span, "thread_id", 0),
        parent_id=parent_id,
        kind=kind,
        location=location,
        attributes=attrs,
        event_attributes=event_attrs,
    )
    event.save()


# Add convenience methods to Span class
def _span_with(name: str, kind: Optional[str] = None):
    """Convenience context manager form.

    Parameters
    ----------
    name : str
        Span name.
    kind : str, optional
        Span kind label.

    Returns
    -------
    Span
        Newly created span (root or child).
    """
    parent = current_span()
    location = _get_location()
    if parent:
        return Span.new_child(parent, name, kind=kind, location=location)
    else:
        return Span(name, kind=kind, location=location)


def _span_decorator(name: Optional[str] = None, kind: Optional[str] = None):
    """Return a decorator that wraps a function in a span.

    Parameters
    ----------
    name : str, optional
        Explicit span name, defaults to function name.
    kind : str, optional
        Kind label.

    Returns
    -------
    Callable
        Decorator applying tracing span.
    """

    def decorator(func: Callable) -> Callable:
        @functools.wraps(func)
        def wrapper(*wargs, **wkwargs):
            span_name = name or func.__name__
            location = _get_location()
            with span_raw(span_name, kind=kind, location=location) as s:
                return func(*wargs, **wkwargs)

        return wrapper

    return decorator


# Monkey-patch Span class with convenience methods
Span.with_ = staticmethod(_span_with)
Span.decorator = staticmethod(_span_decorator)


def add_event(name: str, *, attributes: Optional[list] = None):
    """Add an event to the current span.

    Parameters
    ----------
    name : str
        Event name.
    attributes : list, optional
        Each item is a dict or a (key, value) tuple.

    Raises
    ------
    RuntimeError
        If no span is active.

    Examples
    --------
    >>> with span("op"):
    ...     add_event("phase")
    ...     add_event("kv", attributes=[{"x": 1}, ("y", 2)])
    """
    current = current_span()
    if current is None:
        raise RuntimeError("No active span in current context. Cannot add event.")

    current.add_event(name, attributes=attributes)

    # Record event to table
    _record_event(current, name, attributes)


# Alias for add_event to match the top-level export
event = add_event

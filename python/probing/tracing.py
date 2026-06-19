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
from typing import Callable, Optional

# Import from the internal Rust module
from probing import _core

try:
    Span = _core.Span
    current_span = _core.current_span
    active_span_for_events = _core.active_span_for_events
    active_span_by_kind = _core.active_span_by_kind
    step_snapshot = _core.py_step_snapshot
    sync_local_step = _core.py_sync_local_step
    advance_local_step = _core.py_advance_local_step
    set_step_bucket_size = _core.py_set_step_bucket_size
    current_local_step = _core.py_current_local_step
except AttributeError:
    Span = None

    def current_span():
        return None

    def active_span_for_events():
        return None

    def active_span_by_kind(_kind: str):
        return None

    def step_snapshot():
        return None

    def sync_local_step(_step: int):
        return None

    def advance_local_step():
        return None

    def set_step_bucket_size(_bucket: int):
        return None

    def current_local_step() -> int:
        return 0


from probing.core.table import table

TRAIN_STEP_KIND = "train.step"

# Materialized span rows derived from ``python.trace_event`` (start/end join).
# Use span ``time`` (ns since epoch), not the memtable ingestion ``timestamp``.
SPANS_SQL = """
SELECT
    s.trace_id,
    s.span_id,
    COALESCE(s.parent_id, -1) AS parent_span_id,
    s.name,
    s.kind,
    CAST(s.time / 1000 AS BIGINT) AS start_us,
    CAST(e.time / 1000 AS BIGINT) AS end_us,
    CAST((e.time - s.time) / 1000 AS BIGINT) AS duration_us,
    s.thread_id,
    s.location,
    s.attributes
FROM python.trace_event s
JOIN python.trace_event e
  ON s.span_id = e.span_id AND e.record_type = 'span_end'
WHERE s.record_type = 'span_start'
"""

STAGE_KIND_MAP = {
    "forward": "nn.forward",
    "backward": "nn.backward",
    "step": "optim.step",
}


def _step_fields(snapshot) -> dict:
    if snapshot is None:
        return {}
    return {
        "local_step": int(snapshot.local_step),
        "global_step": int(snapshot.global_step),
        "bucket_size": int(snapshot.bucket_size),
        "rank": int(snapshot.rank),
        "world_size": int(snapshot.world_size),
    }


def _merge_span_attributes(attrs: dict, *, source: str = "manual") -> dict:
    """Merge user attrs with step coordinates, topology, and source label."""
    merged = dict(attrs)
    merged.setdefault("source", source)
    snap = step_snapshot()
    if snap is not None:
        merged.update(_step_fields(snap))
    from probing.parallel import parallel_fields

    merged.update(parallel_fields())
    return merged


def comm_kind(op: str) -> str:
    """Span kind for a collective op, e.g. ``comm.all_reduce``."""
    if op.startswith("comm."):
        return op
    return f"comm.{op}"


def _create_span_object(
    name: str, kind: Optional[str], location: Optional[str], attrs: dict
):
    parent = current_span()
    if parent:
        span_obj = Span.new_child(parent, name, kind=kind, location=location)
    else:
        span_obj = Span(name, kind=kind, location=location)
    if attrs and hasattr(span_obj, "_set_initial_attrs"):
        try:
            span_obj._set_initial_attrs(dict(attrs))
        except Exception as e:
            import warnings

            warnings.warn(f"Failed to set initial attributes: {e}")
    return span_obj


class _RecordedSpan:
    """Internal context manager: span stack + TraceEvent persistence."""

    def __init__(
        self,
        name: str,
        kind: Optional[str] = None,
        location: Optional[str] = None,
        attrs: Optional[dict] = None,
        *,
        source: str = "manual",
    ):
        self.name = name
        self.kind = kind
        self.location = location
        self.attrs = dict(attrs or {})
        self.source = source
        self._span = None
        self._reentrant = False
        self._owns_step_advance = False

    def __enter__(self):
        if self.kind == TRAIN_STEP_KIND:
            existing = active_span_by_kind(TRAIN_STEP_KIND)
            if existing is not None:
                self._span = existing
                self._reentrant = True
                return existing

        loc = self.location or _get_location()
        merged = _merge_span_attributes(self.attrs, source=self.source)
        self._span = _create_span_object(self.name, self.kind, loc, merged)
        self._span.__enter__()
        _record_span_start(self._span, merged)
        if self.kind == TRAIN_STEP_KIND:
            self._owns_step_advance = True
        return self._span

    def __exit__(self, exc_type, exc_val, exc_tb):
        if self._span is None:
            return False
        if self._reentrant:
            return False
        result = self._span.__exit__(exc_type, exc_val, exc_tb)
        _record_span_end(self._span)
        if self._owns_step_advance:
            advance_local_step()
        return result


def recorded_span(name: str, kind: Optional[str] = None, **attrs):
    """Open a span that is always persisted to ``python.trace_event``."""
    return _RecordedSpan(name, kind=kind, attrs=attrs)


def module_stage_kind(stage: str) -> str:
    """Map TorchProbe stage label to span kind."""
    for key, kind in STAGE_KIND_MAP.items():
        if key in stage:
            return kind
    return "torch.module"


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

    if len(args) == 0 and not kwargs:

        def decorator(func: Callable) -> Callable:
            @functools.wraps(func)
            def wrapper(*wargs, **wkwargs):
                with _RecordedSpan(func.__name__, kind=kind) as _s:
                    return func(*wargs, **wkwargs)

            return wrapper

        return decorator

    # Handle @span(func) - first arg is a callable
    if len(args) == 1 and callable(args[0]):
        func = args[0]

        @functools.wraps(func)
        def wrapper(*wargs, **wkwargs):
            with _RecordedSpan(func.__name__, kind=kind) as _s:
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
                self._inner = None

            def __call__(self, func: Callable) -> Callable:
                """Enable decorator form when a name was provided."""

                @functools.wraps(func)
                def wrapper(*wargs, **wkwargs):
                    with _RecordedSpan(
                        self.name,
                        kind=self.kind,
                        location=self.location,
                        attrs=self.attrs,
                    ) as _s:
                        return func(*wargs, **wkwargs)

                return wrapper

            def __enter__(self):
                self._inner = _RecordedSpan(
                    self.name,
                    kind=self.kind,
                    location=self.location,
                    attrs=self.attrs,
                )
                return self._inner.__enter__()

            def __exit__(self, *args):
                if self._inner:
                    return self._inner.__exit__(*args)
                return False

        return SpanWrapper(name, kind, location, kwargs)

    if len(args) > 0:
        name = args[0]
        if not isinstance(name, str):
            raise TypeError("span() requires a string name as the first argument")

        parent = current_span()
        loc = location or _get_location()

        if parent:
            span_obj = Span.new_child(parent, name, kind=kind, location=loc)
        else:
            span_obj = Span(name, kind=kind, location=loc)

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
        trace_id=0,
        span_id=span.span_id,
        name="",
        time=end_ts,
        thread_id=getattr(span, "thread_id", 0),
        parent_id=-1,
        kind="",
        location="",
        attributes="",
        event_attributes="",
    )
    event.save()


def record_closed_span(
    name: str,
    *,
    kind: Optional[str] = None,
    duration_ns: int,
    attrs: Optional[dict] = None,
    source: str = "manual",
) -> None:
    """Persist span_start + span_end without entering the span stack.

    Used for hot-path instrumentation where ``recorded_span`` stack/location
    capture would add unnecessary overhead.
    """
    import json
    import time

    if duration_ns < 0:
        duration_ns = 0

    TraceEvent.init_table()
    merged = _merge_span_attributes(dict(attrs or {}), source=source)
    end_ns = int(time.time_ns())
    start_ns = end_ns - duration_ns

    parent = current_span()
    if parent:
        span_obj = Span.new_child(parent, name, kind=kind, location="")
    else:
        span_obj = Span(name, kind=kind, location="")

    attrs_json = json.dumps(merged) if merged else ""
    parent_id = span_obj.parent_id if span_obj.parent_id is not None else -1
    kind_str = kind or ""

    TraceEvent(
        record_type="span_start",
        trace_id=span_obj.trace_id,
        span_id=span_obj.span_id,
        name=name,
        time=start_ns,
        thread_id=getattr(span_obj, "thread_id", 0),
        parent_id=parent_id,
        kind=kind_str,
        location="",
        attributes=attrs_json,
        event_attributes="",
    ).save()

    TraceEvent(
        record_type="span_end",
        trace_id=0,
        span_id=span_obj.span_id,
        name="",
        time=end_ns,
        thread_id=getattr(span_obj, "thread_id", 0),
        parent_id=-1,
        kind="",
        location="",
        attributes="",
        event_attributes="",
    ).save()


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
            with _RecordedSpan(span_name, kind=kind) as _s:
                return func(*wargs, **wkwargs)

        return wrapper

    return decorator


# Monkey-patch Span class with convenience methods
if Span:
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
    current = active_span_for_events()
    if current is None:
        current = current_span()
    if current is None or getattr(current, "is_ended", False):
        raise RuntimeError("No active span in current context. Cannot add event.")

    current.add_event(name, attributes=attributes)

    # Record event to table
    _record_event(current, name, attributes)


# Alias for add_event to match the top-level export
event = add_event

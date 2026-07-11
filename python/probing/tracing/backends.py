"""Pluggable span backends — fan-out from a single recorder.

Default backend writes ``python.trace_event`` (memtable). Optional backends
include OpenTelemetry export and third-party entry points.

Environment
-----------
``PROBING_SPAN_BACKENDS``
    Comma-separated backend names. Default: ``memtable``.
    Built-in: ``memtable``, ``logger`` (terminal), ``otel`` (requires ``opentelemetry-sdk``),
    ``none`` (stack only). ``configure([])`` also disables persistence until ``reset()``.

``PROBING_SPAN_LOG_LEVEL``
    Log level for the ``logger`` backend (default: ``INFO``).

``OTEL_EXPORTER_OTLP_ENDPOINT`` / standard OTel env vars apply when ``otel`` is enabled.
"""

from __future__ import annotations

import json
import logging
import os
import sys
import time
from dataclasses import dataclass
from typing import Any, Callable, Dict, List, Optional, Protocol, runtime_checkable

logger = logging.getLogger(__name__)

MEMTABLE_BACKEND = "memtable"
LOGGER_BACKEND = "logger"
OTEL_BACKEND = "otel"
NONE_BACKEND = "none"

_trace_event_cls: Any = None
_recorder: Optional["SpanRecorder"] = None
_custom_factories: Dict[str, Callable[[], "SpanBackend"]] = {}
_programmatic_names: Optional[List[str]] = None


@dataclass(frozen=True)
class SpanStartRecord:
    trace_id: int
    span_id: int
    parent_id: int
    name: str
    phase: str
    time_ns: int
    thread_id: int
    location: str
    attributes_json: str


@dataclass(frozen=True)
class SpanEndRecord:
    span_id: int
    time_ns: int
    thread_id: int


@dataclass(frozen=True)
class ClosedSpanRecord:
    start: SpanStartRecord
    end: SpanEndRecord


@dataclass(frozen=True)
class SpanEventRecord:
    trace_id: int
    span_id: int
    parent_id: int
    phase: str
    location: str
    name: str
    time_ns: int
    thread_id: int
    event_attributes_json: str


@runtime_checkable
class SpanBackend(Protocol):
    name: str

    def on_span_start(self, record: SpanStartRecord) -> None: ...

    def on_span_end(self, record: SpanEndRecord) -> None: ...

    def on_span_closed(self, record: ClosedSpanRecord) -> None: ...

    def on_event(self, record: SpanEventRecord) -> None: ...

    def shutdown(self) -> None: ...


def bind_table(trace_event_cls: Any) -> None:
    """Bind the memtable row model (``TraceEvent`` dataclass)."""
    global _trace_event_cls, _recorder
    _trace_event_cls = trace_event_cls
    _recorder = None


def configure(names: Optional[List[str]] = None) -> None:
    """Select span backends by name (overrides ``PROBING_SPAN_BACKENDS`` until ``reset()``)."""
    global _programmatic_names, _recorder
    _programmatic_names = list(names) if names is not None else None
    _recorder = None


def register(name: str, factory: Callable[[], SpanBackend]) -> None:
    """Register a custom backend factory."""
    _custom_factories[name.strip().lower()] = factory
    global _recorder
    _recorder = None


def parse_backend_names(raw: Optional[str] = None) -> List[str]:
    if _programmatic_names is not None:
        return list(_programmatic_names)
    value = (
        raw
        if raw is not None
        else os.environ.get("PROBING_SPAN_BACKENDS", MEMTABLE_BACKEND)
    )
    names = [part.strip().lower() for part in value.split(",") if part.strip()]
    names = [n for n in names if n != NONE_BACKEND]
    if not names and value.strip().lower() == NONE_BACKEND:
        return []
    if not names:
        return [MEMTABLE_BACKEND]
    return names


def _entry_point_backends() -> Dict[str, Callable[[], SpanBackend]]:
    grouped: Dict[str, Callable[[], SpanBackend]] = {}
    try:
        try:
            from importlib.metadata import entry_points as _eps
        except ImportError:
            from importlib_metadata import entry_points as _eps  # type: ignore

        try:
            eps = _eps(group="probing.span_backends")
        except TypeError:
            eps = _eps().get("probing.span_backends", [])
        for ep in eps:
            grouped[ep.name.strip().lower()] = ep.load
    except Exception:
        pass
    return grouped


def _build_memtable_backend() -> SpanBackend:
    if _trace_event_cls is None:
        raise RuntimeError(
            "probing.tracing.backends.bind_table(TraceEvent) was not called"
        )
    return MemtableBackend(_trace_event_cls)


def _build_logger_backend() -> SpanBackend:
    return LoggerBackend()


def _build_otel_backend() -> Optional[SpanBackend]:
    try:
        # API-only installs (opentelemetry-api) are not enough to export spans.
        from opentelemetry.sdk.trace import TracerProvider  # noqa: F401
    except ImportError:
        logger.warning(
            "PROBING_SPAN_BACKENDS includes 'otel' but opentelemetry-sdk is not installed; skipping"
        )
        return None
    return OtelBackend()


def load_backends(names: Optional[List[str]] = None) -> List[SpanBackend]:
    """Instantiate backends for *names* (deduplicated, stable order)."""
    wanted = parse_backend_names(",".join(names) if names else None)
    entry_map = _entry_point_backends()
    out: List[SpanBackend] = []
    seen: set[str] = set()

    for name in wanted:
        if name in seen:
            continue
        seen.add(name)

        backend: Optional[SpanBackend] = None
        if name == MEMTABLE_BACKEND:
            backend = _build_memtable_backend()
        elif name == LOGGER_BACKEND:
            backend = _build_logger_backend()
        elif name == OTEL_BACKEND:
            backend = _build_otel_backend()
        elif name in _custom_factories:
            backend = _custom_factories[name]()
        elif name in entry_map:
            backend = entry_map[name]()
        else:
            logger.warning("Unknown span backend %r — skipped", name)
            continue

        if backend is not None:
            out.append(backend)

    if not out:
        if not wanted:
            return []
        if _programmatic_names is None:
            out.append(_build_memtable_backend())
    return out


class MemtableBackend:
    """Canonical store: ``python.trace_event`` mmap rows."""

    name = MEMTABLE_BACKEND

    def __init__(self, trace_event_cls: Any) -> None:
        self._TraceEvent = trace_event_cls

    def _start_row(self, record: SpanStartRecord):
        return self._TraceEvent(
            record_type="span_start",
            trace_id=record.trace_id,
            span_id=record.span_id,
            name=record.name,
            time=record.time_ns,
            thread_id=record.thread_id,
            parent_id=record.parent_id,
            phase=record.phase,
            location=record.location,
            attributes=record.attributes_json,
            event_attributes="",
        )

    def _end_row(self, record: SpanEndRecord):
        return self._TraceEvent(
            record_type="span_end",
            trace_id=0,
            span_id=record.span_id,
            name="",
            time=record.time_ns,
            thread_id=record.thread_id,
            parent_id=-1,
            phase="",
            location="",
            attributes="",
            event_attributes="",
        )

    def on_span_start(self, record: SpanStartRecord) -> None:
        self._start_row(record).save()

    def on_span_end(self, record: SpanEndRecord) -> None:
        self._end_row(record).save()

    def on_span_closed(self, record: ClosedSpanRecord) -> None:
        self._TraceEvent.append_many(
            [self._start_row(record.start), self._end_row(record.end)]
        )

    def on_event(self, record: SpanEventRecord) -> None:
        self._TraceEvent(
            record_type="event",
            trace_id=record.trace_id,
            span_id=record.span_id,
            name=record.name,
            time=record.time_ns,
            thread_id=record.thread_id,
            parent_id=record.parent_id,
            phase=record.phase,
            location=record.location,
            attributes="",
            event_attributes=record.event_attributes_json,
        ).save()

    def shutdown(self) -> None:
        return None


def _terminal_logger() -> logging.Logger:
    """Logger that prints span lines to stderr when no handler is configured."""
    log = logging.getLogger("probing.span")
    if not log.handlers:
        handler = logging.StreamHandler(sys.stderr)
        handler.setFormatter(logging.Formatter("%(message)s"))
        log.addHandler(handler)
        level = os.environ.get("PROBING_SPAN_LOG_LEVEL", "INFO").upper()
        log.setLevel(getattr(logging, level, logging.INFO))
        log.propagate = False
    return log


class LoggerBackend:
    """Print span lifecycle to the terminal (works alongside other backends)."""

    name = LOGGER_BACKEND

    def __init__(self, log: Optional[logging.Logger] = None) -> None:
        self._log = log or _terminal_logger()
        self._depth = 0
        self._open: Dict[int, tuple[str, int]] = {}

    def _indent(self) -> str:
        return "  " * self._depth

    def on_span_start(self, record: SpanStartRecord) -> None:
        self._open[record.span_id] = (record.name, record.time_ns)
        parts = [f"→ {record.name}"]
        if record.phase:
            parts.append(f"phase={record.phase}")
        source = _attr_from_json(record.attributes_json, "source")
        if source:
            parts.append(f"source={source}")
        self._log.info("%s%s", self._indent(), " ".join(parts))
        self._depth += 1

    def on_span_end(self, record: SpanEndRecord) -> None:
        self._depth = max(0, self._depth - 1)
        opened = self._open.pop(record.span_id, None)
        if opened is not None:
            name, start_ns = opened
            dur_ms = max(0.0, (record.time_ns - start_ns) / 1e6)
            self._log.info("%s← %s %.2fms", self._indent(), name, dur_ms)
        else:
            self._log.info("%s← span_id=%s", self._indent(), record.span_id)

    def on_span_closed(self, record: ClosedSpanRecord) -> None:
        self.on_span_start(record.start)
        self.on_span_end(record.end)

    def on_event(self, record: SpanEventRecord) -> None:
        suffix = ""
        if record.event_attributes_json:
            try:
                parsed = json.loads(record.event_attributes_json)
                if isinstance(parsed, dict) and parsed:
                    suffix = " " + json.dumps(parsed, ensure_ascii=False)
            except json.JSONDecodeError:
                suffix = f" {record.event_attributes_json}"
        self._log.info("%s· %s%s", self._indent(), record.name, suffix)

    def shutdown(self) -> None:
        self._open.clear()
        self._depth = 0


def _attr_from_json(raw: str, key: str) -> Optional[str]:
    if not raw:
        return None
    try:
        parsed = json.loads(raw)
    except json.JSONDecodeError:
        return None
    if not isinstance(parsed, dict):
        return None
    value = parsed.get(key)
    return str(value) if value is not None else None


class OtelBackend:
    """Optional OpenTelemetry export (Jaeger/Grafana/OTLP via standard OTel env)."""

    name = OTEL_BACKEND

    def __init__(self) -> None:
        from opentelemetry import trace
        from opentelemetry.trace import SpanKind, set_span_in_context

        self._trace = trace
        self._SpanKind = SpanKind
        self._set_span_in_context = set_span_in_context
        self._tracer = trace.get_tracer("probing")
        self._spans: Dict[int, Any] = {}
        self._parents: Dict[int, int] = {}

    def _kind(self, kind: str) -> Any:
        mapping = {
            "server": self._SpanKind.SERVER,
            "client": self._SpanKind.CLIENT,
            "producer": self._SpanKind.PRODUCER,
            "consumer": self._SpanKind.CONSUMER,
        }
        return mapping.get(kind, self._SpanKind.INTERNAL)

    def on_span_start(self, record: SpanStartRecord) -> None:
        parent_ctx = None
        if record.parent_id not in (-1, None):
            parent_otel = self._spans.get(record.parent_id)
            if parent_otel is not None:
                parent_ctx = self._set_span_in_context(parent_otel)

        otel_span = self._tracer.start_span(
            record.name,
            context=parent_ctx,
            kind=self._kind(record.phase),
            start_time=record.time_ns,
        )
        if record.attributes_json:
            try:
                attrs = json.loads(record.attributes_json)
                if isinstance(attrs, dict):
                    for key, value in attrs.items():
                        otel_span.set_attribute(str(key), value)
                else:
                    otel_span.set_attribute(
                        "probing.attributes", record.attributes_json
                    )
            except json.JSONDecodeError:
                otel_span.set_attribute("probing.attributes", record.attributes_json)
        if record.phase:
            otel_span.set_attribute("probing.phase", record.phase)
        if record.location:
            otel_span.set_attribute("probing.location", record.location)

        self._spans[record.span_id] = otel_span
        self._parents[record.span_id] = record.parent_id

    def on_span_end(self, record: SpanEndRecord) -> None:
        otel_span = self._spans.pop(record.span_id, None)
        self._parents.pop(record.span_id, None)
        if otel_span is None:
            return
        otel_span.end(end_time=record.time_ns)

    def on_event(self, record: SpanEventRecord) -> None:
        otel_span = self._spans.get(record.span_id)
        if otel_span is None:
            return
        attrs: Dict[str, Any] = {}
        if record.event_attributes_json:
            try:
                parsed = json.loads(record.event_attributes_json)
                if isinstance(parsed, dict):
                    attrs = {str(k): v for k, v in parsed.items()}
            except json.JSONDecodeError:
                attrs = {"raw": record.event_attributes_json}
        otel_span.add_event(record.name, attributes=attrs, timestamp=record.time_ns)

    def shutdown(self) -> None:
        for span_id, otel_span in list(self._spans.items()):
            try:
                otel_span.end()
            except Exception:
                pass
            self._spans.pop(span_id, None)
        self._parents.clear()


def _span_phase(span: Any) -> str:
    return str(getattr(span, "phase", None) or "")


def _span_location(span: Any) -> str:
    loc = getattr(span, "location", None)
    return str(loc) if loc is not None else ""


def _parent_id(span: Any) -> int:
    pid = span.parent_id
    return int(pid if pid is not None else -1)


def _thread_id(span: Any) -> int:
    return int(getattr(span, "thread_id", 0))


class SpanRecorder:
    """Fan-out span lifecycle records to all enabled backends."""

    def __init__(self, backends: List[SpanBackend]) -> None:
        self._backends = backends

    @property
    def enabled(self) -> bool:
        return bool(self._backends)

    @property
    def backend_names(self) -> List[str]:
        return [b.name for b in self._backends]

    def record_span_start(self, span: Any, attrs: dict) -> None:
        if not self.enabled:
            return
        record = _span_start_record(span, attrs)
        self._dispatch("on_span_start", record)

    def record_span_end(self, span: Any) -> None:
        if not self.enabled:
            return
        end_ts = span.end_timestamp or int(time.time_ns())
        record = SpanEndRecord(
            span_id=int(span.span_id),
            time_ns=int(end_ts),
            thread_id=_thread_id(span),
        )
        self._dispatch("on_span_end", record)

    def record_closed_span(
        self,
        span: Any,
        *,
        name: str,
        phase: str,
        start_ns: int,
        end_ns: int,
        attributes_json: str,
    ) -> None:
        if not self.enabled:
            return
        start = SpanStartRecord(
            trace_id=int(span.trace_id),
            span_id=int(span.span_id),
            parent_id=_parent_id(span),
            name=name,
            phase=phase,
            time_ns=int(start_ns),
            thread_id=_thread_id(span),
            location=_span_location(span),
            attributes_json=attributes_json,
        )
        end = SpanEndRecord(
            span_id=int(span.span_id),
            time_ns=int(end_ns),
            thread_id=_thread_id(span),
        )
        self._dispatch_closed(start, end)

    def record_event(
        self, span: Any, event_name: str, event_attributes: Optional[list] = None
    ) -> None:
        if not self.enabled:
            return
        record = _event_record(span, event_name, event_attributes)
        self._dispatch("on_event", record)

    def shutdown(self) -> None:
        for backend in self._backends:
            try:
                backend.shutdown()
            except Exception as exc:
                logger.debug("span backend %s.shutdown failed: %s", backend.name, exc)

    def _dispatch(self, method: str, record: Any) -> None:
        for backend in self._backends:
            _safe_call(backend, method, record)

    def _dispatch_closed(self, start: SpanStartRecord, end: SpanEndRecord) -> None:
        closed = ClosedSpanRecord(start=start, end=end)
        for backend in self._backends:
            if hasattr(backend, "on_span_closed"):
                _safe_call(backend, "on_span_closed", closed)
            else:
                _safe_call(backend, "on_span_start", start)
                _safe_call(backend, "on_span_end", end)


def _safe_call(backend: SpanBackend, method: str, record: Any) -> None:
    try:
        getattr(backend, method)(record)
    except Exception as exc:
        logger.debug("span backend %s.%s failed: %s", backend.name, method, exc)


def _span_start_record(span: Any, attrs: dict) -> SpanStartRecord:
    return SpanStartRecord(
        trace_id=int(span.trace_id),
        span_id=int(span.span_id),
        parent_id=_parent_id(span),
        name=str(span.name),
        phase=_span_phase(span),
        time_ns=int(span.start_timestamp),
        thread_id=_thread_id(span),
        location=_span_location(span),
        attributes_json=json.dumps(attrs) if attrs else "",
    )


def _event_record(
    span: Any, event_name: str, event_attributes: Optional[list]
) -> SpanEventRecord:
    attrs_dict: Dict[str, Any] = {}
    if event_attributes:
        for attr_item in event_attributes:
            if isinstance(attr_item, dict):
                attrs_dict.update(attr_item)
            elif isinstance(attr_item, (list, tuple)) and len(attr_item) == 2:
                attrs_dict[attr_item[0]] = attr_item[1]
    return SpanEventRecord(
        trace_id=int(span.trace_id),
        span_id=int(span.span_id),
        parent_id=_parent_id(span),
        phase=_span_phase(span),
        location=_span_location(span),
        name=str(event_name),
        time_ns=int(time.time_ns()),
        thread_id=_thread_id(span),
        event_attributes_json=json.dumps(attrs_dict) if attrs_dict else "",
    )


def get_recorder(*, reset: bool = False) -> SpanRecorder:
    global _recorder
    if _recorder is None or reset:
        _recorder = SpanRecorder(load_backends())
    return _recorder


def persistence_enabled() -> bool:
    """True when at least one span backend will receive lifecycle records."""
    return get_recorder().enabled


def _reset_recorder() -> None:
    """Drop cached recorder instance."""
    global _recorder
    if _recorder is not None:
        try:
            _recorder.shutdown()
        except Exception:
            pass
    _recorder = None


def reset(*, clear_registered: bool = False) -> None:
    """Drop cached recorder; optionally clear ``register()`` factories and ``configure()`` override."""
    global _programmatic_names
    if clear_registered:
        _custom_factories.clear()
    _programmatic_names = None
    _reset_recorder()


def list_backends() -> List[str]:
    """Return names of currently active span backends."""
    return get_recorder().backend_names


__all__ = [
    "SpanBackend",
    "MemtableBackend",
    "LoggerBackend",
    "OtelBackend",
    "SpanRecorder",
    "register",
    "configure",
    "list_backends",
    "reset",
    "bind_table",
]

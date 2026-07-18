"""
Ray integration for probing using OpenTelemetry.

This module provides automatic tracing of Ray tasks and actors using OpenTelemetry,
with integration to probing for data storage and querying.

Usage:
    import ray
    ray.init(_tracing_startup_hook="probing.ext.ray:setup_tracing")

    @ray.remote
    def my_task(x):
        return x * 2

    result = my_task.remote(5)
"""

import functools
import hashlib
import os
import socket
from typing import Any, Optional

from probing.utils.py import _get_attr, _get_ray


RAY_PROCESS_FIELDS = [
    "timestamp_ns",
    "pid",
    "hostname",
    "node_ip",
    "ray_job_id",
    "ray_node_id",
    "ray_worker_id",
    "ray_task_id",
    "ray_actor_id",
    "ray_actor_name",
    "ray_namespace",
    "process_role",
]

RAY_TASK_FIELDS = [
    "timestamp_ns",
    "task_id",
    "parent_task_id",
    "actor_id",
    "function_name",
    "task_type",
    "state",
    "job_id",
    "node_id",
    "worker_id",
    "worker_pid",
    "start_time_ns",
    "end_time_ns",
]

RAY_ACTOR_FIELDS = [
    "timestamp_ns",
    "actor_id",
    "class_name",
    "state",
    "job_id",
    "node_id",
    "worker_id",
    "worker_pid",
    "start_time_ns",
    "end_time_ns",
]

_ray_tables: dict[str, Any] = {}
_span_context_enabled = False


def _ray_table(name: str, fields: list[str]):
    import probing

    table = _ray_tables.get(name)
    if table is not None:
        return table

    try:
        table = probing.ExternalTable.get(name)
        if table.names() != fields:
            probing.ExternalTable.drop(name)
            table = probing.ExternalTable.get_or_create(name, fields)
    except Exception:
        table = probing.ExternalTable.get_or_create(name, fields)
    _ray_tables[name] = table
    return table


def _append_ray_row(name: str, fields: list[str], row: dict[str, Any]) -> None:
    try:
        _ray_table(name, fields).append([row.get(field, "") for field in fields])
    except Exception:
        pass


def _now_ns() -> int:
    import time

    return time.time_ns()


def _stable_int(value: str, modulo: int = 2_000_000_000) -> int:
    if not value:
        return 1
    digest = hashlib.blake2b(value.encode("utf-8"), digest_size=8).digest()
    return int.from_bytes(digest, "big") % modulo + 1


def _safe_str(value: Any) -> str:
    if value is None:
        return ""
    try:
        return str(value)
    except Exception:
        return ""


def _safe_int(value: Any, default: int = 0) -> int:
    if value is None or value == "":
        return default
    try:
        return int(value)
    except Exception:
        return default


def _call_attr(obj: Any, names: list[str], default: Any = "") -> Any:
    for name in names:
        try:
            attr = getattr(obj, name, None)
            if attr is None:
                continue
            value = attr() if callable(attr) else attr
            if value is not None:
                return value
        except Exception:
            continue
    return default


def _current_ray_context_attrs() -> dict[str, str]:
    """Return Ray runtime identity for the current driver/worker process."""
    attrs: dict[str, str] = {}
    try:
        ray = _get_ray()
        if not ray.is_initialized():
            return attrs
        ctx = ray.get_runtime_context()
    except Exception:
        return attrs

    for key, names in {
        "ray_job_id": ["get_job_id", "job_id"],
        "ray_node_id": ["get_node_id", "node_id"],
        "ray_namespace": ["namespace"],
    }.items():
        value = _call_attr(ctx, names)
        if value:
            attrs[key] = _safe_str(value)

    is_worker = False
    try:
        worker_module = getattr(getattr(ray, "_private", None), "worker", None)
        global_worker = getattr(worker_module, "global_worker", None)
        worker_mode = getattr(worker_module, "WORKER_MODE", None)
        is_worker = (
            global_worker is not None
            and worker_mode is not None
            and getattr(global_worker, "mode", None) == worker_mode
        )
    except Exception:
        is_worker = False

    if is_worker:
        for key, names in {
            "ray_worker_id": ["get_worker_id", "worker_id"],
            "ray_task_id": ["get_task_id", "task_id"],
            "ray_actor_id": ["get_actor_id", "actor_id"],
        }.items():
            value = _call_attr(ctx, names)
            if value:
                attrs[key] = _safe_str(value)

        actor_name = _call_attr(ctx, ["get_actor_name", "actor_name"])
        if actor_name:
            attrs["ray_actor_name"] = _safe_str(actor_name)

    return attrs


def _process_identity_attrs(process_role: str = "") -> dict[str, Any]:
    try:
        hostname = socket.gethostname()
    except Exception:
        hostname = ""

    # Optional framework adapters bridge their env into PROBING_* once.
    try:
        from probing.ext.slime import apply_env_bridge

        apply_env_bridge()
    except Exception:
        pass

    node_ip = (
        os.environ.get("PROBING_NODE_IP")
        or os.environ.get("POD_IP")
        or os.environ.get("RAY_NODE_IP")
        or ""
    )
    if not node_ip and hostname:
        try:
            node_ip = socket.gethostbyname(hostname)
        except Exception:
            node_ip = ""

    attrs: dict[str, Any] = {
        "pid": os.getpid(),
        "hostname": hostname,
        "node_ip": node_ip,
    }
    attrs.update(_current_ray_context_attrs())
    role = (
        process_role
        or os.environ.get("PROBING_RAY_PROCESS_ROLE", "")
        or os.environ.get("PROBING_PROCESS_ROLE", "")
    )
    if role:
        attrs["process_role"] = role
    return attrs


def current_process_identity(process_role: str = "") -> dict[str, Any]:
    """Return the current process identity used to correlate Ray spans."""
    return _process_identity_attrs(process_role)


def register_current_process(process_role: str = "") -> None:
    """Persist the current Ray process identity, if Ray/probing are available."""
    try:
        attrs = _process_identity_attrs(process_role)
        _append_ray_row(
            "ray_process",
            RAY_PROCESS_FIELDS,
            {
                "timestamp_ns": _now_ns(),
                "pid": os.getpid(),
                "hostname": _safe_str(attrs.get("hostname")),
                "node_ip": _safe_str(attrs.get("node_ip")),
                "ray_job_id": _safe_str(attrs.get("ray_job_id")),
                "ray_node_id": _safe_str(attrs.get("ray_node_id")),
                "ray_worker_id": _safe_str(attrs.get("ray_worker_id")),
                "ray_task_id": _safe_str(attrs.get("ray_task_id")),
                "ray_actor_id": _safe_str(attrs.get("ray_actor_id")),
                "ray_actor_name": _safe_str(attrs.get("ray_actor_name")),
                "ray_namespace": _safe_str(attrs.get("ray_namespace")),
                "process_role": _safe_str(attrs.get("process_role")),
            },
        )
    except Exception:
        pass


def _ray_span_attributes() -> dict[str, Any]:
    """Attributes added to manual probing spans in Ray driver/worker processes."""
    return _process_identity_attrs()


def enable_span_context() -> None:
    """Attach Ray process identity to every manual ``probing.span`` in this process."""
    global _span_context_enabled
    if _span_context_enabled:
        return
    try:
        from probing.tracing import add_span_attribute_provider

        add_span_attribute_provider(_ray_span_attributes)
        _span_context_enabled = True
    except Exception:
        pass


def setup_driver(process_role: str = "driver") -> None:
    """Enable Ray span attributes and record the current driver process identity."""
    if process_role:
        os.environ["PROBING_RAY_PROCESS_ROLE"] = process_role
    enable_span_context()
    register_current_process(process_role=process_role)


class ProbingSpanProcessor:
    """OpenTelemetry SpanProcessor that converts spans to probing."""

    def __init__(self):
        self._probing_available = False
        try:
            import probing

            self._probing_available = True
        except ImportError:
            pass
        self._span_map = {}

    def on_start(self, span, parent_context=None):
        """Create probing span when OpenTelemetry span starts."""
        if not self._probing_available:
            return

        try:
            from opentelemetry.trace import SpanKind

            import probing

            span_name = span.name

            kind_map = {
                SpanKind.SERVER: "server",
                SpanKind.CLIENT: "client",
                SpanKind.INTERNAL: "internal",
                SpanKind.PRODUCER: "producer",
                SpanKind.CONSUMER: "consumer",
            }
            attrs = {}
            if hasattr(span, "attributes") and span.attributes:
                attrs = {str(k): str(v) for k, v in span.attributes.items()}
            span_kind = kind_map.get(span.kind)
            if span_kind:
                attrs["otel.span_kind"] = span_kind
            attrs.update({k: _safe_str(v) for k, v in _process_identity_attrs().items()})

            span_context = span.get_span_context()
            attrs["otel_trace_id"] = format(span_context.trace_id, "032x")
            attrs["otel_span_id"] = format(span_context.span_id, "016x")
            try:
                parent = getattr(span, "parent", None)
                if parent:
                    attrs["otel_parent_span_id"] = format(parent.span_id, "016x")
            except Exception:
                pass

            probing_span = probing.span(span_name, **attrs)
            probing_span.__enter__()

            self._span_map[(span_context.trace_id, span_context.span_id)] = probing_span
        except Exception:
            pass

    def on_end(self, span):
        """End probing span when OpenTelemetry span ends."""
        if not self._probing_available:
            return

        try:
            span_context = span.get_span_context()
            span_key = (span_context.trace_id, span_context.span_id)
            probing_span = self._span_map.pop(span_key, None)
            if probing_span:
                probing_span.__exit__(None, None, None)
        except Exception:
            pass

    def shutdown(self):
        """Shutdown the processor."""
        self._span_map.clear()

    def force_flush(self, timeout_millis: int = 30000):
        """Force flush any pending spans."""
        pass


def _wrap_task_execution_in_worker():
    """Wrap Ray task execution to add OpenTelemetry tracing."""
    try:
        from opentelemetry import trace
        from opentelemetry.trace import Status, StatusCode

        ray = _get_ray()
        tracer = trace.get_tracer(__name__)

        if not hasattr(ray, "worker") or not hasattr(ray.worker, "execute_task"):
            return

        worker = ray.worker
        original_execute = worker.execute_task

        @functools.wraps(original_execute)
        def traced_execute(*args, **kwargs):
            span_name = "ray.task"
            attributes = {}

            try:
                task = getattr(worker, "current_task", None)
                if task:
                    if hasattr(task, "actor_id") and task.actor_id:
                        actor_name = getattr(task, "actor_class_name", "Actor")
                        method_name = getattr(task, "function_name", "method")
                        span_name = f"{actor_name}.{method_name}"
                        attributes["ray.actor"] = actor_name
                        attributes["ray.method"] = method_name
                    else:
                        func_name = (
                            getattr(task, "function_name", None)
                            or getattr(task, "name", None)
                            or "unknown"
                        )
                        span_name = func_name
                        attributes["ray.function"] = func_name
            except Exception:
                pass

            with tracer.start_as_current_span(
                span_name, kind=trace.SpanKind.INTERNAL, attributes=attributes
            ) as span:
                try:
                    result = original_execute(*args, **kwargs)
                    span.set_status(Status(StatusCode.OK))
                    return result
                except Exception as e:
                    span.set_status(Status(StatusCode.ERROR, str(e)))
                    span.record_exception(e)
                    raise

        worker.execute_task = traced_execute
    except Exception:
        pass


def init():
    """Initialize Ray tracing integration (called by import hook)."""
    enable_span_context()


def setup_tracing() -> None:
    """Tracing startup hook called in each Ray worker.

    This function is called by Ray when each worker process starts.
    It sets up OpenTelemetry tracing and exports spans to probing.
    """
    try:
        from opentelemetry import trace
        from opentelemetry.sdk.trace import TracerProvider

        os.environ["PROBING"] = os.environ.get("PROBING", "1")
        try:
            from probing.ext.slime import apply_env_bridge

            apply_env_bridge()
        except Exception:
            pass
        worker_role = (
            os.environ.get("PROBING_RAY_PROCESS_ROLE", "").strip()
            or os.environ.get("PROBING_PROCESS_ROLE", "").strip()
            or "ray_worker"
        )
        os.environ["PROBING_RAY_PROCESS_ROLE"] = worker_role
        enable_span_context()

        trace.set_tracer_provider(TracerProvider())
        trace.get_tracer_provider().add_span_processor(ProbingSpanProcessor())

        register_current_process(process_role=worker_role)
        _wrap_task_execution_in_worker()
    except Exception:
        pass


def _worker_pid(worker_info) -> int:
    return _safe_int(
        _get_attr(
            worker_info,
            [
                "pid",
                "worker_pid",
                "process_id",
                "processId",
            ],
        )
    )


def _worker_id(worker_info) -> str:
    return _safe_str(_get_attr(worker_info, ["worker_id", "workerId", "id"], ""))


def _worker_node_id(worker_info) -> str:
    return _safe_str(_get_attr(worker_info, ["node_id", "nodeId"], ""))


def _collect_worker_info() -> dict[str, dict[str, Any]]:
    """Return worker_id -> state metadata from Ray state API when available."""
    try:
        from ray.util.state import list_workers

        workers = list(list_workers(detail=True))
    except Exception:
        return {}

    result: dict[str, dict[str, Any]] = {}
    for worker in workers:
        worker_id = _worker_id(worker)
        if not worker_id:
            continue
        result[worker_id] = {
            "pid": _worker_pid(worker),
            "node_id": _worker_node_id(worker),
            "worker_type": _safe_str(_get_attr(worker, ["worker_type", "type"], "")),
        }
    return result


def _process_id_for_worker(worker_id: str, node_id: str = "", worker_pid: int = 0) -> int:
    if worker_pid > 0:
        return _stable_int(f"{node_id}:{worker_pid}")
    if worker_id:
        return _stable_int(worker_id)
    return 1


def _extract_time_from_events(events):
    """Extract start and end time from Ray task events."""
    if not events:
        return None, None

    start_time_ms = None
    end_time_ms = None

    for event in events:
        # Events are typically dict-like objects with 'event_type' and 'time_ms' fields
        event_type = _get_attr(event, ["event_type", "type"])
        time_ms = _get_attr(event, ["time_ms", "timestamp_ms", "timestamp"])

        if not time_ms:
            continue

        # Convert to milliseconds if needed
        if time_ms < 1e10:  # Likely seconds
            time_ms = time_ms * 1000
        elif time_ms > 1e15:  # Likely microseconds
            time_ms = time_ms / 1000

        event_type_str = str(event_type).upper() if event_type else ""

        # Find start event (TASK_STARTED, RUNNING, etc.)
        if not start_time_ms and (
            "START" in event_type_str
            or "RUNNING" in event_type_str
            or "SCHEDULED" in event_type_str
        ):
            start_time_ms = time_ms

        # Find end event (TASK_FINISHED, FAILED, etc.)
        if (
            "FINISH" in event_type_str
            or "FAIL" in event_type_str
            or "CANCEL" in event_type_str
            or "END" in event_type_str
        ):
            if not end_time_ms or time_ms > end_time_ms:
                end_time_ms = time_ms

    return start_time_ms, end_time_ms


def _convert_task_to_timeline_entry(task, index=0, total=1, worker_info=None):
    """Convert Ray TaskState to timeline entry.

    Note: If start_time_ms and end_time_ms are None, we use a relative timeline
    based on task order. This happens when Ray timeline recording is not enabled
    or tasks have already been cleaned up from GCS.

    Parameters
    ----------
    task : TaskState
        Ray task state object
    index : int
        Task index for relative timeline fallback
    total : int
        Total number of tasks for relative timeline fallback
    worker_info : dict, optional
        Mapping from worker_id to process metadata.
    """
    task_id = _get_attr(task, ["task_id"], "")
    func_name = _get_attr(
        task,
        ["func_or_class_name", "function_name", "name"],
        "unknown_task",
    )

    # Try to get time from events first (most reliable)
    events = _get_attr(task, ["events"])
    start_time_ms, end_time_ms = _extract_time_from_events(events)

    # Fallback to direct time fields
    if start_time_ms is None:
        start_time_ms = _get_attr(task, ["start_time_ms", "creation_time_ms"])
    if end_time_ms is None:
        end_time_ms = _get_attr(task, ["end_time_ms"])

    # If still no time info, use relative timeline based on task order
    # This is a fallback when Ray timeline recording is not available
    if start_time_ms is None:
        # Use current time minus a relative offset based on task index
        import time

        current_time_ms = time.time() * 1000
        # Assume tasks are spread over 1 second, with each task taking ~10ms
        start_time_ms = current_time_ms - (total - index) * 10
        end_time_ms = start_time_ms + 10  # Default 10ms duration

    # Convert milliseconds to nanoseconds
    start_time_ns = int(start_time_ms * 1_000_000) if start_time_ms else None
    end_time_ns = int(end_time_ms * 1_000_000) if end_time_ms else None
    duration = (
        (end_time_ns - start_time_ns) if (start_time_ns and end_time_ns) else None
    )

    # Determine task type
    task_type = _get_attr(task, ["type"], "")
    actor_id = _get_attr(task, ["actor_id"])
    is_actor_task = actor_id is not None or "ACTOR" in str(task_type)

    # Get worker_id and determine process_id (pid)
    worker_id = _get_attr(task, ["worker_id"], "")
    node_id = _safe_str(_get_attr(task, ["node_id"], ""))
    info = (worker_info or {}).get(worker_id, {}) if worker_id else {}
    worker_pid = _safe_int(info.get("pid"))
    if info.get("node_id") and not node_id:
        node_id = _safe_str(info.get("node_id"))
    process_id = _process_id_for_worker(_safe_str(worker_id), node_id, worker_pid)

    attributes = {
        "task_id": str(task_id),
        "function_name": func_name,
        "state": _get_attr(task, ["state"], "unknown"),
        "worker_id": str(worker_id),
        "worker_pid": str(worker_pid),
        "node_id": node_id,
        "job_id": str(_get_attr(task, ["job_id"], "")),
        "task_type": str(task_type),
    }

    if actor_id:
        attributes["actor_id"] = str(actor_id)

    parent_task_id = _get_attr(task, ["parent_task_id"])
    # Filter out the default parent task ID
    if (
        parent_task_id
        and str(parent_task_id) != "ffffffffffffffffffffffffffffffffffffffff01000000"
    ):
        attributes["parent_task_id"] = str(parent_task_id)

    # Determine entry name and type
    if is_actor_task:
        entry_name = func_name
        entry_type = "actor"
    else:
        entry_name = func_name
        entry_type = "task"

    return {
        "name": entry_name,
        "type": entry_type,
        "start_time": start_time_ns or 0,
        "end_time": end_time_ns,
        "duration": duration,
        "trace_id": hash(task_id) if task_id else 0,
        "span_id": hash(task_id) if task_id else 0,
        "parent_id": (
            hash(parent_task_id)
            if parent_task_id
            and str(parent_task_id)
            != "ffffffffffffffffffffffffffffffffffffffff01000000"
            else None
        ),
        "kind": entry_type,
        "thread_id": 0,
        "process_id": process_id,  # Add process_id for Chrome tracing format
        "worker_pid": worker_pid,
        "attributes": attributes,
    }


def _convert_actor_to_timeline_entry(actor, worker_info=None):
    """Convert Ray actor to timeline entry.

    Parameters
    ----------
    actor : ActorState
        Ray actor state object
    worker_info : dict, optional
        Mapping from worker_id to process metadata.
    """
    actor_id = _get_attr(actor, ["actor_id"], "")
    class_name = _get_attr(
        actor,
        ["class_name", "name"],
        "unknown_actor",
    )

    # Try to get time from events
    events = _get_attr(actor, ["events"])
    start_time_ms, end_time_ms = _extract_time_from_events(events)

    # Fallback to direct time fields
    if start_time_ms is None:
        start_time_ms = _get_attr(actor, ["start_time_ms", "creation_time_ms"])
    if end_time_ms is None:
        end_time_ms = _get_attr(actor, ["end_time_ms"])

    # Convert milliseconds to nanoseconds
    start_time_ns = int(start_time_ms * 1_000_000) if start_time_ms else None
    end_time_ns = int(end_time_ms * 1_000_000) if end_time_ms else None
    duration = (
        (end_time_ns - start_time_ns) if (start_time_ns and end_time_ns) else None
    )

    # Get worker_id and determine process_id (pid)
    worker_id = _get_attr(actor, ["worker_id"], "")
    node_id = _safe_str(_get_attr(actor, ["node_id"], ""))
    info = (worker_info or {}).get(worker_id, {}) if worker_id else {}
    worker_pid = _safe_int(info.get("pid"))
    if info.get("node_id") and not node_id:
        node_id = _safe_str(info.get("node_id"))
    process_id = _process_id_for_worker(_safe_str(worker_id), node_id, worker_pid)

    attributes = {
        "actor_id": str(actor_id),
        "class_name": class_name,
        "state": _get_attr(actor, ["state"], "unknown"),
        "worker_id": str(worker_id),
        "worker_pid": str(worker_pid),
        "node_id": node_id,
        "job_id": str(_get_attr(actor, ["job_id"], "")),
    }

    return {
        "name": f"ray.actor.{class_name}",
        "type": "actor",
        "start_time": start_time_ns or 0,
        "end_time": end_time_ns,
        "duration": duration,
        "trace_id": hash(actor_id) if actor_id else 0,
        "span_id": hash(actor_id) if actor_id else 0,
        "parent_id": None,
        "kind": "actor",
        "thread_id": 0,
        "process_id": process_id,  # Add process_id for Chrome tracing format
        "worker_pid": worker_pid,
        "attributes": attributes,
    }


def _save_ray_task_entry(entry: dict[str, Any]) -> None:
    try:
        attrs = entry.get("attributes", {})
        _append_ray_row(
            "ray_task",
            RAY_TASK_FIELDS,
            {
                "timestamp_ns": _now_ns(),
                "task_id": _safe_str(attrs.get("task_id")),
                "parent_task_id": _safe_str(attrs.get("parent_task_id")),
                "actor_id": _safe_str(attrs.get("actor_id")),
                "function_name": _safe_str(attrs.get("function_name")),
                "task_type": _safe_str(attrs.get("task_type")),
                "state": _safe_str(attrs.get("state")),
                "job_id": _safe_str(attrs.get("job_id")),
                "node_id": _safe_str(attrs.get("node_id")),
                "worker_id": _safe_str(attrs.get("worker_id")),
                "worker_pid": _safe_int(attrs.get("worker_pid")),
                "start_time_ns": _safe_int(entry.get("start_time")),
                "end_time_ns": _safe_int(entry.get("end_time")),
            },
        )
    except Exception:
        pass


def _save_ray_actor_entry(entry: dict[str, Any]) -> None:
    try:
        attrs = entry.get("attributes", {})
        _append_ray_row(
            "ray_actor",
            RAY_ACTOR_FIELDS,
            {
                "timestamp_ns": _now_ns(),
                "actor_id": _safe_str(attrs.get("actor_id")),
                "class_name": _safe_str(attrs.get("class_name")),
                "state": _safe_str(attrs.get("state")),
                "job_id": _safe_str(attrs.get("job_id")),
                "node_id": _safe_str(attrs.get("node_id")),
                "worker_id": _safe_str(attrs.get("worker_id")),
                "worker_pid": _safe_int(attrs.get("worker_pid")),
                "start_time_ns": _safe_int(entry.get("start_time")),
                "end_time_ns": _safe_int(entry.get("end_time")),
            },
        )
    except Exception:
        pass


def get_ray_timeline(
    task_filter: Optional[str] = None,
    actor_filter: Optional[str] = None,
    start_time: Optional[int] = None,
    end_time: Optional[int] = None,
) -> list:
    """Get Ray task execution timeline using Ray's state API.

    Parameters
    ----------
    task_filter : str, optional
        Filter tasks by function name pattern.
    actor_filter : str, optional
        Filter actors by class name pattern.
    start_time : int, optional
        Start time in nanoseconds since epoch.
    end_time : int, optional
        End time in nanoseconds since epoch.

    Returns
    -------
    list
        List of timeline entries.
    """
    try:
        ray = _get_ray()
        if not ray.is_initialized():
            return []

        from ray.util.state import list_actors, list_tasks

        timeline: list[dict] = []

        task_filters = {"func_or_class_name": task_filter} if task_filter else {}
        actor_filters = {"class_name": actor_filter} if actor_filter else {}

        try:
            tasks_iter = list_tasks(filters=task_filters or None, detail=True)
            tasks_list = list(tasks_iter)
        except Exception:
            tasks_list = []

        try:
            actors_iter = list_actors(filters=actor_filters or None, detail=True)
            actors_list = list(actors_iter)
        except Exception:
            actors_list = []

        worker_info = _collect_worker_info()

        # Second pass: convert tasks with worker metadata mapping
        total_tasks = len(tasks_list)

        for index, task in enumerate(tasks_list):
            entry = _convert_task_to_timeline_entry(
                task, index, total_tasks, worker_info
            )

            # Apply time filters
            if start_time and entry["start_time"] and entry["start_time"] < start_time:
                continue
            if end_time and entry["end_time"] and entry["end_time"] > end_time:
                continue
            _save_ray_task_entry(entry)
            timeline.append(entry)

        # Convert actors with worker metadata mapping
        for actor in actors_list:
            entry = _convert_actor_to_timeline_entry(actor, worker_info)
            # Apply time filters
            if start_time and entry["start_time"] and entry["start_time"] < start_time:
                continue
            if end_time and entry["end_time"] and entry["end_time"] > end_time:
                continue
            _save_ray_actor_entry(entry)
            timeline.append(entry)

        timeline.sort(key=lambda x: x["start_time"])
        return timeline

    except Exception:
        return []


def get_ray_timeline_chrome_format(
    task_filter: Optional[str] = None,
    actor_filter: Optional[str] = None,
    start_time: Optional[int] = None,
    end_time: Optional[int] = None,
) -> str:
    """Get Ray timeline in Chrome tracing format.

    Returns JSON string that can be viewed in chrome://tracing or perfetto.
    Each process represents a different worker, with process name showing worker and node info.
    """
    try:
        import json

        timeline = get_ray_timeline(task_filter, actor_filter, start_time, end_time)
        if not timeline:
            return json.dumps({"traceEvents": []})

        earliest_time = min(
            entry["start_time"] for entry in timeline if entry["start_time"]
        )

        # Build worker_id to info mapping from timeline entries
        worker_to_info = {}
        for entry in timeline:
            attributes = entry.get("attributes", {})
            worker_id = attributes.get("worker_id", "")
            if worker_id and worker_id not in worker_to_info:
                worker_to_info[worker_id] = {
                    "node_id": attributes.get("node_id", ""),
                    "worker_pid": _safe_int(attributes.get("worker_pid")),
                }

        # Build process_id to worker_id reverse mapping and update worker info
        pid_to_worker = {}
        for entry in timeline:
            process_id = entry.get("process_id", 1)
            attributes = entry.get("attributes", {})
            worker_id = attributes.get("worker_id", "")
            if worker_id:
                if process_id not in pid_to_worker:
                    pid_to_worker[process_id] = worker_id
                # Update worker info with node_id from this entry if available
                if worker_id in worker_to_info:
                    node_id = attributes.get("node_id", "")
                    if node_id and not worker_to_info[worker_id]["node_id"]:
                        worker_to_info[worker_id]["node_id"] = node_id
                    worker_pid = _safe_int(attributes.get("worker_pid"))
                    if worker_pid and not worker_to_info[worker_id]["worker_pid"]:
                        worker_to_info[worker_id]["worker_pid"] = worker_pid

        trace_events = []

        # Add process name metadata events (must come before other events)
        # Chrome tracing format uses "M" (Metadata) events with "process_name" to name processes
        for process_id, worker_id in pid_to_worker.items():
            worker_info = worker_to_info.get(worker_id, {})
            node_id = worker_info.get("node_id", "")
            worker_pid = worker_info.get("worker_pid")

            # Build process name with worker and node info
            process_name_parts = []
            if worker_id:
                # Use short worker_id for display (first 8 chars)
                short_worker_id = worker_id[:8] if len(worker_id) > 8 else worker_id
                process_name_parts.append(f"Worker:{short_worker_id}")
            if node_id:
                # Use short node_id for display (first 8 chars)
                short_node_id = node_id[:8] if len(node_id) > 8 else node_id
                process_name_parts.append(f"Node:{short_node_id}")
            if worker_pid:
                process_name_parts.append(f"PID:{worker_pid}")

            process_name = (
                " | ".join(process_name_parts)
                if process_name_parts
                else f"Worker {process_id}"
            )

            # Add process name metadata event
            trace_events.append(
                {
                    "name": "process_name",
                    "ph": "M",
                    "pid": process_id,
                    "args": {"name": process_name},
                }
            )

            # Add process labels with full info
            process_labels = []
            if worker_id:
                process_labels.append(f"worker_id={worker_id}")
            if node_id:
                process_labels.append(f"node_id={node_id}")
            if worker_pid:
                process_labels.append(f"worker_pid={worker_pid}")

            if process_labels:
                trace_events.append(
                    {
                        "name": "process_labels",
                        "ph": "M",
                        "pid": process_id,
                        "args": {"labels": ", ".join(process_labels)},
                    }
                )

        # Add task/actor events
        for entry in timeline:
            # Use process_id from entry, fallback to 1 if not present
            process_id = entry.get("process_id", 1)

            trace_events.append(
                {
                    "name": entry["name"],
                    "cat": entry["type"],
                    "ph": "B",
                    "ts": (entry["start_time"] - earliest_time) / 1000,
                    "pid": process_id,
                    "tid": entry.get("thread_id", 0),
                    "args": entry.get("attributes", {}),
                }
            )

            if entry["end_time"]:
                trace_events.append(
                    {
                        "name": entry["name"],
                        "cat": entry["type"],
                        "ph": "E",
                        "ts": (entry["end_time"] - earliest_time) / 1000,
                        "pid": process_id,
                        "tid": entry.get("thread_id", 0),
                    }
                )

        return json.dumps(
            {"traceEvents": trace_events, "displayTimeUnit": "ms"}, indent=2
        )
    except Exception:
        import json

        return json.dumps({"traceEvents": []})

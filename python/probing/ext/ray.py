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
import os
from typing import Optional

from probing.utils.py import _get_attr, _get_ray


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
            import probing
            from opentelemetry.trace import SpanKind

            span_name = span.name

            kind_map = {
                SpanKind.SERVER: "server",
                SpanKind.CLIENT: "client",
                SpanKind.INTERNAL: "internal",
                SpanKind.PRODUCER: "producer",
                SpanKind.CONSUMER: "consumer",
            }
            span_kind = kind_map.get(span.kind)

            attrs = {}
            if hasattr(span, "attributes") and span.attributes:
                attrs = {str(k): str(v) for k, v in span.attributes.items()}

            probing_span = probing.span(span_name, kind=span_kind, **attrs)
            probing_span.__enter__()

            span_context = span.get_span_context()
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
    # Users should explicitly use: ray.init(_tracing_startup_hook="probing.ext.ray:setup_tracing")
    pass


def setup_tracing() -> None:
    """Tracing startup hook called in each Ray worker.

    This function is called by Ray when each worker process starts.
    It sets up OpenTelemetry tracing and exports spans to probing.
    """
    try:
        from opentelemetry import trace
        from opentelemetry.sdk.trace import TracerProvider

        trace.set_tracer_provider(TracerProvider())
        trace.get_tracer_provider().add_span_processor(ProbingSpanProcessor())

        os.environ["PROBING"] = os.environ.get("PROBING", "1")
        _wrap_task_execution_in_worker()
    except Exception:
        pass


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


def _convert_task_to_timeline_entry(task, index=0, total=1, worker_to_pid=None):
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
    worker_to_pid : dict, optional
        Mapping from worker_id to process_id (pid). If None, will use hash of worker_id.
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
    if worker_to_pid is not None and worker_id:
        process_id = worker_to_pid.get(worker_id, hash(worker_id) % 10000 + 1)
    elif worker_id:
        # Use hash of worker_id to generate a consistent pid
        process_id = abs(hash(worker_id)) % 10000 + 1
    else:
        process_id = 1  # Default pid for tasks without worker_id

    attributes = {
        "task_id": str(task_id),
        "function_name": func_name,
        "state": _get_attr(task, ["state"], "unknown"),
        "worker_id": str(worker_id),
        "node_id": str(_get_attr(task, ["node_id"], "")),
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
        "attributes": attributes,
    }


def _convert_actor_to_timeline_entry(actor, worker_to_pid=None):
    """Convert Ray actor to timeline entry.

    Parameters
    ----------
    actor : ActorState
        Ray actor state object
    worker_to_pid : dict, optional
        Mapping from worker_id to process_id (pid). If None, will use hash of worker_id.
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
    if worker_to_pid is not None and worker_id:
        process_id = worker_to_pid.get(worker_id, hash(worker_id) % 10000 + 1)
    elif worker_id:
        # Use hash of worker_id to generate a consistent pid
        process_id = abs(hash(worker_id)) % 10000 + 1
    else:
        process_id = 1  # Default pid for actors without worker_id

    attributes = {
        "actor_id": str(actor_id),
        "class_name": class_name,
        "state": _get_attr(actor, ["state"], "unknown"),
        "worker_id": str(worker_id),
        "node_id": str(_get_attr(actor, ["node_id"], "")),
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
        "attributes": attributes,
    }


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

        from ray.util.state import list_tasks, list_actors

        timeline: list[dict] = []

        # Build worker_id -> process_id mapping in a first pass
        worker_ids: set[str] = set()

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

        for task in tasks_list:
            worker_id = _get_attr(task, "worker_id", "")
            if worker_id:
                worker_ids.add(worker_id)

        for actor in actors_list:
            worker_id = _get_attr(actor, "worker_id", "")
            if worker_id:
                worker_ids.add(worker_id)

        worker_to_pid = {
            worker_id: idx for idx, worker_id in enumerate(sorted(worker_ids), start=1)
        }

        # Second pass: convert tasks with worker_to_pid mapping
        total_tasks = len(tasks_list)

        for index, task in enumerate(tasks_list):
            entry = _convert_task_to_timeline_entry(
                task, index, total_tasks, worker_to_pid
            )

            # Apply time filters
            if start_time and entry["start_time"] and entry["start_time"] < start_time:
                continue
            if end_time and entry["end_time"] and entry["end_time"] > end_time:
                continue
            timeline.append(entry)

        # Convert actors with worker_to_pid mapping
        for actor in actors_list:
            entry = _convert_actor_to_timeline_entry(actor, worker_to_pid)
            # Apply time filters
            if start_time and entry["start_time"] and entry["start_time"] < start_time:
                continue
            if end_time and entry["end_time"] and entry["end_time"] > end_time:
                continue
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
                    "worker_pid": None,  # worker_pid is not in attributes, we'll try to get it from task
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

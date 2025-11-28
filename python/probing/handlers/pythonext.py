"""Python extension API handlers.

This module contains handler functions for various API endpoints
that were previously embedded as Python code strings in Rust.
"""

import json
import sys
import io
import traceback
from typing import Optional, List, Dict, Any

from probing.handlers.router import ext_handler, handle_request


@ext_handler(
    "pythonext",
    [
        "ray/timeline/chrome",
        "python/ray/timeline/chrome",
        "pythonext/ray/timeline/chrome",
    ],
)
def get_ray_timeline_chrome_format(
    task_filter: Optional[str] = None,
    actor_filter: Optional[str] = None,
    start_time: Optional[int] = None,
    end_time: Optional[int] = None,
) -> str:
    """Get Ray timeline in Chrome tracing format.

    Args:
        task_filter: Optional task filter string
        actor_filter: Optional actor filter string
        start_time: Optional start time in nanoseconds
        end_time: Optional end time in nanoseconds

    Returns:
        JSON string containing Chrome tracing format data
    """
    try:
        from probing.ext.ray import get_ray_timeline_chrome_format as _get_chrome

        chrome_trace = _get_chrome(
            task_filter=task_filter,
            actor_filter=actor_filter,
            start_time=start_time,
            end_time=end_time,
        )

        return chrome_trace
    except Exception as e:
        error_msg = str(e)
        error_trace = traceback.format_exc()
        return json.dumps({"error": error_msg, "traceback": error_trace})


@ext_handler(
    "pythonext",
    [
        "ray/timeline",
        "python/ray/timeline",
        "pythonext/ray/timeline",
    ],
)
def get_ray_timeline(
    task_filter: Optional[str] = None,
    actor_filter: Optional[str] = None,
    start_time: Optional[int] = None,
    end_time: Optional[int] = None,
) -> str:
    """Get Ray timeline data.

    Args:
        task_filter: Optional task filter string
        actor_filter: Optional actor filter string
        start_time: Optional start time in nanoseconds
        end_time: Optional end time in nanoseconds

    Returns:
        JSON string containing timeline data
    """
    try:
        from probing.ext.ray import get_ray_timeline as _get_timeline

        timeline = _get_timeline(
            task_filter=task_filter,
            actor_filter=actor_filter,
            start_time=start_time,
            end_time=end_time,
        )

        return json.dumps(timeline)
    except Exception as e:
        error_msg = str(e)
        error_trace = traceback.format_exc()
        return json.dumps({"error": error_msg, "traceback": error_trace})


@ext_handler("pythonext", "trace/chrome-tracing")
def get_chrome_tracing(limit: int = 1000) -> str:
    """Convert trace events to Chrome tracing format.

    Args:
        limit: Maximum number of events to process (0 for no limit)

    Returns:
        JSON string containing Chrome tracing format data
    """
    import probing.core.engine as engine

    try:
        # Query trace events from the database
        # IMPORTANT: Order by timestamp ASC to process events in chronological order
        # This ensures span_start events are processed before their corresponding span_end events
        limit_clause = f" LIMIT {limit}" if limit > 0 else ""
        query = f"""
            SELECT 
                record_type,
                trace_id,
                span_id,
                COALESCE(parent_id, -1) as parent_id,
                name,
                time as timestamp,
                COALESCE(thread_id, 0) as thread_id,
                kind,
                location,
                attributes,
                event_attributes
            FROM python.trace_event
            ORDER BY timestamp ASC
            {limit_clause}
        """

        df = engine.query(query)

        # Convert DataFrame to Chrome tracing format
        trace_events = []
        # Check if DataFrame is not None and not empty
        # Use df is not None and not df.empty instead of if df (ambiguous truth value)
        if df is not None and not df.empty:
            # Convert DataFrame to list of dictionaries for iteration
            df_list = df.to_dict("records") if hasattr(df, "to_dict") else []
            # Find minimum timestamp
            timestamps = [
                row.get("timestamp", 0) for row in df_list if "timestamp" in row
            ]
            min_timestamp = min(timestamps) if timestamps else 0

            # Track span starts by (span_id, thread_id) to handle multiple threads
            # Also track trace_id for span_end events (which may have trace_id=0)
            span_starts = {}

            # First pass: collect all span_start events to build a lookup table
            # This helps match span_end events even if trace_id is 0 in span_end
            span_start_lookup = {}
            for row in df_list:
                if row.get("record_type") == "span_start":
                    span_id = row.get("span_id", 0)
                    thread_id = row.get("thread_id", 0)
                    trace_id = row.get("trace_id", 0)
                    name = row.get("name", "unknown")
                    kind = row.get("kind", "trace")
                    # Use (span_id, thread_id) as key to handle multiple threads
                    key = (span_id, thread_id)
                    span_start_lookup[key] = {
                        "trace_id": trace_id,
                        "name": name,
                        "kind": kind,
                        "timestamp": row.get("timestamp", 0),
                    }

            # Second pass: convert events to Chrome tracing format
            for row in df_list:
                record_type = row.get("record_type", "")
                timestamp = row.get("timestamp", 0)
                name = row.get("name", "unknown")
                trace_id = row.get("trace_id", 0)
                span_id = row.get("span_id", 0)
                thread_id = row.get("thread_id", 0)
                kind = row.get("kind", "trace")

                # Convert nanoseconds to microseconds
                ts_micros = (timestamp - min_timestamp) // 1000
                # Use trace_id from span_start if available, otherwise use current trace_id
                pid = trace_id
                tid = thread_id

                if record_type == "span_start":
                    # Store span start information with trace_id for matching
                    key = (span_id, thread_id)
                    span_starts[key] = (ts_micros, name, kind, pid)
                    chrome_event = {
                        "name": name,
                        "cat": kind if kind else "span",
                        "ph": "B",
                        "ts": ts_micros,
                        "pid": pid,
                        "tid": tid,
                    }
                    if row.get("location"):
                        chrome_event["args"] = {"location": row.get("location")}
                    trace_events.append(chrome_event)
                elif record_type == "span_end":
                    # Try to find matching span_start
                    key = (span_id, thread_id)
                    start_info = span_starts.get(key)

                    if start_info:
                        # Found matching span_start that was already processed
                        start_ts, start_name, start_kind, start_pid = start_info
                        # Use the pid from span_start to ensure matching
                        chrome_event = {
                            "name": start_name,  # Must match span_start name
                            "cat": (
                                start_kind if start_kind else "span"
                            ),  # Must match span_start cat
                            "ph": "E",
                            "ts": ts_micros,
                            "pid": start_pid,  # Use pid from span_start
                            "tid": tid,  # Must match span_start tid
                        }
                        # Note: Chrome tracing B/E events don't need dur, but we can add it for debugging
                        dur = ts_micros - start_ts
                        if dur > 0:
                            chrome_event["dur"] = dur
                        trace_events.append(chrome_event)
                        # Remove from span_starts to avoid duplicate matches
                        del span_starts[key]
                    else:
                        # No matching span_start found in processed events, try lookup
                        lookup_info = span_start_lookup.get(key)
                        if lookup_info:
                            # Use trace_id and other info from span_start
                            start_pid = lookup_info["trace_id"]
                            start_ts = (
                                lookup_info["timestamp"] - min_timestamp
                            ) // 1000
                            start_name = lookup_info["name"]
                            start_kind = lookup_info["kind"]
                            chrome_event = {
                                "name": start_name,  # Must match span_start name
                                "cat": (
                                    start_kind if start_kind else "span"
                                ),  # Must match span_start cat
                                "ph": "E",
                                "ts": ts_micros,
                                "pid": start_pid,  # Use pid from span_start
                                "tid": tid,  # Must match span_start tid
                            }
                            dur = ts_micros - start_ts
                            if dur > 0:
                                chrome_event["dur"] = dur
                            trace_events.append(chrome_event)
                        else:
                            # No matching span_start found at all
                            # This might happen if span_start was filtered out by limit
                            # Create a standalone end event with warning
                            chrome_event = {
                                "name": name if name else "unknown_span",
                                "cat": "span",
                                "ph": "E",
                                "ts": ts_micros,
                                "pid": (
                                    pid if pid > 0 else 1
                                ),  # Use current pid or default
                                "tid": tid,
                            }
                            trace_events.append(chrome_event)
                elif record_type == "event":
                    chrome_event = {
                        "name": name,
                        "cat": "event",
                        "ph": "i",
                        "ts": ts_micros,
                        "pid": pid,
                        "tid": tid,
                        "s": "t",
                    }
                    if row.get("event_attributes"):
                        try:
                            chrome_event["args"] = json.loads(
                                row.get("event_attributes")
                            )
                        except:
                            pass
                    trace_events.append(chrome_event)

        chrome_trace = {"traceEvents": trace_events, "displayTimeUnit": "ms"}
        return json.dumps(chrome_trace, indent=2)
    except Exception as e:
        return json.dumps(
            {"error": str(e), "trace": traceback.format_exc(), "traceEvents": []}
        )


@ext_handler("pythonext", "pytorch/timeline")
def get_pytorch_timeline() -> str:
    """Get PyTorch profiler timeline.

    Returns:
        JSON string containing timeline data
    """
    try:
        # Use the same approach as REPL - call _cmd_timeline() method
        # This ensures we use the exact same code path that works in REPL
        from probing.repl.torch_magic import TorchMagic
        import __main__

        shell = None  # TorchMagic doesn't actually need shell for _cmd_timeline
        torch_magic = TorchMagic(shell)

        # Check if profiler exists before calling _cmd_timeline
        # This avoids printing non-JSON messages to stdout
        if not hasattr(__main__, "__probing__"):
            return json.dumps(
                {
                    "error": "No profiler found. Use 'pytorch/profile' API to start profiler first."
                }
            )

        profiler = __main__.__probing__.get(TorchMagic.PROFILER_KEY)
        if profiler is None:
            return json.dumps(
                {
                    "error": "No profiler found. Use 'pytorch/profile' API to start profiler first."
                }
            )

        # Capture stdout to get the timeline JSON output
        old_stdout = sys.stdout
        sys.stdout = captured_output = io.StringIO()

        try:
            # Call the timeline method (same as REPL)
            torch_magic._cmd_timeline()

            # Get the captured output
            timeline_output = captured_output.getvalue()
            sys.stdout = old_stdout

            if not timeline_output or timeline_output.strip() == "":
                return json.dumps(
                    {
                        "error": "No timeline data available. Make sure the profiler has been executed."
                    }
                )

            # Check if output is an error message (non-JSON text)
            output_stripped = timeline_output.strip()
            if output_stripped.startswith("No ") or output_stripped.startswith(
                "No timeline"
            ):
                # This is an error message, not JSON
                return json.dumps({"error": output_stripped})

            # Try to parse as JSON
            try:
                timeline_data = json.loads(output_stripped)
                # Return the timeline data directly
                return json.dumps(timeline_data, indent=2)
            except json.JSONDecodeError as e:
                # If parsing fails, check if it's an error message
                if "No profiler" in output_stripped or "No timeline" in output_stripped:
                    return json.dumps({"error": output_stripped})
                # Otherwise return error with the raw output for debugging
                return json.dumps(
                    {
                        "error": f"Failed to parse timeline output: {str(e)}",
                        "raw_output": output_stripped[:500],
                    }
                )
        except Exception as e:
            sys.stdout = old_stdout
            return json.dumps(
                {
                    "error": f"Failed to get timeline: {str(e)}",
                    "traceback": traceback.format_exc(),
                }
            )
    except Exception as e:
        return json.dumps(
            {
                "error": f"Failed to initialize TorchMagic: {str(e)}",
                "traceback": traceback.format_exc(),
            }
        )


@ext_handler("pythonext", "pytorch/profile")
def start_pytorch_profile(steps: int = 1) -> str:
    """Start PyTorch global profiler.

    Args:
        steps: Number of steps to profile

    Returns:
        JSON string with success status
    """
    try:
        # Use global profiler - call _start_global_profiler via _cmd_profile
        from probing.repl.torch_magic import TorchMagic

        shell = None
        torch_magic = TorchMagic(shell)
        torch_magic._start_global_profiler(steps)
        return json.dumps(
            {"success": True, "message": f"Global profiler started for {steps} step(s)"}
        )
    except Exception as e:
        return json.dumps(
            {"success": False, "error": str(e), "traceback": traceback.format_exc()}
        )


def get_pytorch_profile() -> str:
    """Get PyTorch profiler profile data.

    Returns:
        JSON string containing profile data
    """
    # This function can be implemented if needed
    # For now, returning a placeholder
    return json.dumps({"error": "Not implemented"})


@ext_handler("pythonext", "trace/list")
def list_trace(prefix: Optional[str] = None) -> str:
    """List traceable functions.

    Args:
        prefix: Optional prefix to filter functions

    Returns:
        JSON string containing list of traceable functions
    """
    try:
        from probing.inspect.trace import list_traceable

        result = list_traceable(prefix=prefix)
        return result if result else "[]"
    except Exception as e:
        return json.dumps({"error": str(e)})


@ext_handler("pythonext", "trace/show")
def show_trace() -> str:
    """Show current trace configuration.

    Returns:
        JSON string containing trace configuration
    """
    try:
        from probing.inspect.trace import show_trace as _show_trace

        result = _show_trace()
        return result if result else "[]"
    except Exception as e:
        return json.dumps({"error": str(e)})


@ext_handler("pythonext", "trace/start", required_params=["function"])
def start_trace(
    function: str,
    watch: Optional[List[str]] = None,
    silent_watch: Optional[List[str]] = None,
    depth: int = 1,
    print_to_terminal: bool = False,
) -> str:
    """Start tracing a function.

    Args:
        function: Function name to trace
        watch: List of variables to watch (comma-separated string from params)
        silent_watch: List of variables to watch silently (usually not set directly)
        depth: Trace depth
        print_to_terminal: If True, use watch; if False, use silent_watch

    Returns:
        JSON string with success status
    """
    try:
        from probing.inspect.trace import trace

        # Determine whether to use watch or silent_watch based on print_to_terminal
        # This matches the original Rust logic
        if print_to_terminal:
            watch_list = watch or []
            silent_watch_list = []
        else:
            watch_list = []
            silent_watch_list = watch or []

        trace(function, watch=watch_list, silent_watch=silent_watch_list, depth=depth)
        return json.dumps({"success": True, "message": f"Started tracing {function}"})
    except Exception as e:
        return json.dumps({"success": False, "error": str(e)})


@ext_handler("pythonext", "trace/stop", required_params=["function"])
def stop_trace(function: str) -> str:
    """Stop tracing a function.

    Args:
        function: Function name to stop tracing

    Returns:
        JSON string with success status
    """
    try:
        from probing.inspect.trace import untrace

        untrace(function)
        return json.dumps({"success": True, "message": f"Stopped tracing {function}"})
    except Exception as e:
        return json.dumps({"success": False, "error": str(e)})


@ext_handler("pythonext", "trace/variables")
def get_trace_variables(function: Optional[str] = None, limit: int = 100) -> str:
    """Get trace variables from database.

    Args:
        function: Optional function name to filter by
        limit: Maximum number of records to return

    Returns:
        JSON string containing trace variables
    """
    try:
        import probing

        # Try with python namespace first, fallback to direct table name
        if function:
            queries = [
                f"SELECT function_name, filename, lineno, variable_name, value, value_type, timestamp FROM python.trace_variables WHERE function_name = '{function}' ORDER BY timestamp DESC LIMIT {limit}",
                f"SELECT function_name, filename, lineno, variable_name, value, value_type, timestamp FROM trace_variables WHERE function_name = '{function}' ORDER BY timestamp DESC LIMIT {limit}",
            ]
        else:
            queries = [
                f"SELECT function_name, filename, lineno, variable_name, value, value_type, timestamp FROM python.trace_variables ORDER BY timestamp DESC LIMIT {limit}",
                f"SELECT function_name, filename, lineno, variable_name, value, value_type, timestamp FROM trace_variables ORDER BY timestamp DESC LIMIT {limit}",
            ]

        df = None
        for query in queries:
            try:
                df = probing.query(query)
                break
            except:
                continue

        if df is None:
            return json.dumps({"error": "Table trace_variables not found"})
        else:
            result = df.to_dict("records")
            return json.dumps(result)
    except Exception as e:
        return json.dumps({"error": str(e)})


# Unified entry point for all handlers
def handle_api_request(path: str, params: Dict[str, str]) -> str:
    """Unified entry point for handling API requests.

    This function routes requests to the appropriate handler based on the path.
    All parameter parsing and validation is handled automatically.

    Handlers are automatically registered via the @ext_handler decorator when the module is imported.

    Args:
        path: API path (e.g., "ray/timeline", "trace/list")
        params: Query parameters as dictionary of strings

    Returns:
        JSON string response
    """
    return handle_request(path, params)

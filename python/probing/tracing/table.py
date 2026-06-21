"""Trace event table schema (SQL / federation)."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Optional

from probing.core.table import table

# Materialized span rows derived from ``python.trace_event`` (start/end join).
# Use span ``time`` (ns since epoch), not the memtable ingestion ``timestamp``.
SPANS_SQL = """
SELECT
    s.trace_id,
    s.span_id,
    COALESCE(s.parent_id, -1) AS parent_span_id,
    s.name,
    s.phase,
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


@table
@dataclass
class TraceEvent:
    """Row model for trace records.

    Each saved instance is one of: span_start, span_end, event.
    """

    record_type: str
    trace_id: int
    span_id: int
    name: str
    time: int
    thread_id: int = 0
    parent_id: Optional[int] = -1
    phase: Optional[str] = ""
    location: Optional[str] = ""
    attributes: Optional[str] = ""
    event_attributes: Optional[str] = ""

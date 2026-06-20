"""Collective recording: comm_collective rows and optional trace_event mirror."""

import json

import pytest

from probing.profiling.collective.record import (
    CommCollective,
    begin_comm_span,
    finish_comm_span,
    record_comm_lite,
)
from probing.tracing import TraceEvent

from .conftest import table_rows


@pytest.mark.training_observability
class TestCommCollectiveRecording:
    def test_full_mode_span_writes_row_and_trace(self):
        cm, meta = begin_comm_span(
            "all_reduce",
            group_rank=0,
            group_size=8,
            participate_ranks=list(range(8)),
            tensor_shape="(1024,)",
            tensor_dtype="torch.float32",
            nbytes=4096,
            async_op=False,
        )
        finish_comm_span(
            cm,
            meta,
            op="all_reduce",
            duration_ms=12.5,
            group_rank=0,
            group_size=8,
        )

        rows = table_rows(CommCollective, 10)
        assert len(rows) == 1
        assert rows[0]["op"] == "all_reduce"
        assert rows[0]["group_size"] == 8
        assert rows[0]["duration_ms"] == pytest.approx(12.5)
        assert json.loads(rows[0]["participate_ranks"]) == list(range(8))

        events = table_rows(TraceEvent, 10)
        assert len(events) >= 2
        starts = [e for e in events if e["record_type"] == "span_start"]
        assert any(e["kind"] == "comm.all_reduce" for e in starts)

    def test_lite_mode_writes_comm_row(self):
        record_comm_lite(
            op="all_reduce",
            duration_ms=3.5,
            group_rank=1,
            group_size=4,
            nbytes=1024,
        )

        rows = table_rows(CommCollective, 10)
        assert len(rows) == 1
        assert rows[0]["op"] == "all_reduce"
        assert rows[0]["duration_ms"] == pytest.approx(3.5)
        assert rows[0]["participate_ranks"] == ""

    def test_lite_mode_writes_closed_trace_pair_by_default(self):
        record_comm_lite(
            op="all_reduce",
            duration_ms=2.0,
            group_rank=0,
            group_size=2,
            nbytes=512,
        )

        events = table_rows(TraceEvent, 10)
        by_type = {row["record_type"]: row for row in events}
        assert by_type["span_start"]["name"] == "all_reduce"
        assert by_type["span_start"]["kind"] == "comm.all_reduce"
        assert by_type["span_end"]["span_id"] == by_type["span_start"]["span_id"]

    def test_lite_mode_can_skip_trace_event(self):
        record_comm_lite(
            op="broadcast",
            duration_ms=1.0,
            group_rank=0,
            group_size=2,
            write_trace_event=False,
        )

        assert len(table_rows(CommCollective, 10)) == 1
        assert len(table_rows(TraceEvent, 10)) == 0

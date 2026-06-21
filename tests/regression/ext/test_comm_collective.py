import dataclasses
import json

import pytest

from probing.profiling.collective.record import (
    CommCollective,
    finish_comm_span,
    begin_comm_span,
)
from probing.tracing import TraceEvent


@pytest.fixture(autouse=True)
def _reset_tables():
    for table_cls in (CommCollective, TraceEvent):
        try:
            table_cls.drop()
        except Exception:
            pass
        table_cls.init_table()
    yield


def test_comm_collective_row_saved():
    cm, meta = begin_comm_span(
        "all_reduce",
        group_rank=0,
        group_size=8,
        participate_ranks=[0, 1, 2, 3, 4, 5, 6, 7],
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

    rows = CommCollective.take(10)
    assert len(rows) == 1
    _ts, data = rows[0]
    row = dict(zip([f.name for f in dataclasses.fields(CommCollective)], data))
    assert row["op"] == "all_reduce"
    assert row["group_size"] == 8
    assert row["duration_ms"] == pytest.approx(12.5)
    assert json.loads(row["participate_ranks"]) == list(range(8))


def test_comm_lite_row_saved():
    from probing.profiling.collective.record import record_comm_lite

    CommCollective.init_table()
    record_comm_lite(
        op="all_reduce",
        duration_ms=3.5,
        group_rank=1,
        group_size=4,
        nbytes=1024,
    )

    rows = CommCollective.take(10)
    assert len(rows) == 1
    _ts, data = rows[0]
    row = dict(zip([f.name for f in dataclasses.fields(CommCollective)], data))
    assert row["op"] == "all_reduce"
    assert row["duration_ms"] == pytest.approx(3.5)
    assert row["group_size"] == 4
    assert row["participate_ranks"] == ""


def test_comm_lite_writes_trace_event():
    from probing.profiling.collective.record import record_comm_lite

    CommCollective.init_table()
    TraceEvent.init_table()
    record_comm_lite(
        op="all_reduce",
        duration_ms=2.0,
        group_rank=0,
        group_size=2,
        nbytes=512,
        write_trace_event=True,
    )

    events = TraceEvent.take(10)
    assert len(events) == 2
    rows = [
        dict(zip([f.name for f in dataclasses.fields(TraceEvent)], data))
        for _ts, data in events
    ]
    by_type = {row["record_type"]: row for row in rows}
    assert by_type["span_start"]["name"] == "all_reduce"
    assert by_type["span_end"]["span_id"] == by_type["span_start"]["span_id"]

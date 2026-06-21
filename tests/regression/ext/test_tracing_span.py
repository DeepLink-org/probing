import dataclasses
import time

import pytest

import probing


@pytest.fixture(autouse=True)
def _reset_trace_event_table():
    """Isolate memtable rows so persistence tests are deterministic."""
    from probing.tracing import TraceEvent, bind_table, reset_backends
    from probing.tracing.phases import reset_phase

    reset_phase()

    try:
        TraceEvent.drop()
    except Exception:
        pass
    TraceEvent.init_table()
    reset_backends(clear_registered=True)
    bind_table(TraceEvent)
    yield
    reset_backends(clear_registered=True)


def _trace_event_rows(n: int = 50) -> list[dict]:
    from probing.tracing import TraceEvent

    fields = [f.name for f in dataclasses.fields(TraceEvent)]
    return [dict(zip(fields, data)) for _ts, data in TraceEvent.take(n)]


def _span_duration_ns(rows: list[dict], span_id: int) -> int | None:
    starts = {r["span_id"]: r for r in rows if r.get("record_type") == "span_start"}
    ends = {r["span_id"]: r for r in rows if r.get("record_type") == "span_end"}
    start = starts.get(span_id)
    end = ends.get(span_id)
    if start is None or end is None:
        return None
    return int(end["time"]) - int(start["time"])


def test_context_manager_basic():
    with probing.span("root") as s:
        assert s.name == "root"
        assert s.status == "Active"
        assert not s.is_ended
    assert s.status == "Completed"
    assert s.is_ended


def test_decorator_named_and_plain():
    @probing.span("decor_named")
    def f1():
        return 1

    @probing.span  # implicit name from function
    def f2():
        return 2

    assert f1() == 1
    assert f2() == 2


def test_nested_parent_child_ids():
    with probing.span("parent") as parent:
        assert parent.parent_id is None
        with probing.span("child") as child:
            assert child.parent_id == parent.span_id
            assert child.trace_id == parent.trace_id
            with probing.span("grandchild") as grandchild:
                assert grandchild.parent_id == child.span_id
                assert grandchild.trace_id == parent.trace_id


def test_current_span_stack_behavior():
    from probing.tracing import current_span

    assert current_span() is None
    with probing.span("a") as a:
        top = current_span()
        assert top is not None
        assert top.span_id == a.span_id
        with probing.span("b") as b:
            top2 = current_span()
            assert top2.span_id == b.span_id
        # after inner exits
        again = current_span()
        assert again.span_id == a.span_id
    assert current_span() is None


def test_property_immutability():
    with probing.span("immutable", phase="forward") as s:
        original_id = s.span_id
        with pytest.raises(AttributeError):
            s.name = "changed"
        with pytest.raises(AttributeError):
            s.phase = "other"
        with pytest.raises(AttributeError):
            s.span_id = 123
        assert s.span_id == original_id
        assert s.name == "immutable"
        assert s.phase == "forward"


def test_events_recording():
    with probing.span("events") as s:
        s.add_event("e1")
        s.add_event("e2", attributes=[{"k": "v"}])
        events = s.get_events()
        assert len(events) == 2
        assert events[0]["name"] == "e1"
        assert events[1]["name"] == "e2"
        assert events[1]["attributes"]["k"] == "v"

    rows = _trace_event_rows()
    event_rows = [r for r in rows if r.get("record_type") == "event"]
    assert len(event_rows) == 2
    assert {r["name"] for r in event_rows} == {"e1", "e2"}


def test_span_persists_start_end_pair_to_trace_event():
    with probing.span("persist_me", phase="forward") as s:
        span_id = s.span_id
        trace_id = s.trace_id

    rows = _trace_event_rows()
    starts = [r for r in rows if r.get("record_type") == "span_start"]
    ends = [r for r in rows if r.get("record_type") == "span_end"]
    assert len(starts) == 1
    assert len(ends) == 1
    assert starts[0]["span_id"] == span_id
    assert starts[0]["trace_id"] == trace_id
    assert starts[0]["name"] == "persist_me"
    assert starts[0]["phase"] == "forward"
    assert ends[0]["span_id"] == span_id
    duration_ns = _span_duration_ns(rows, span_id)
    assert duration_ns is not None
    assert duration_ns >= 0


def test_nested_spans_persist_parent_links():
    with probing.span("parent") as parent:
        with probing.span("child") as child:
            child_id = child.span_id
            parent_id = parent.span_id

    rows = _trace_event_rows()
    child_start = next(
        r
        for r in rows
        if r.get("record_type") == "span_start" and r.get("span_id") == child_id
    )
    assert child_start["parent_id"] == parent_id


def test_decorator_persists_trace_event_rows():
    @probing.span("decor_persist")
    def work():
        return 7

    assert work() == 7
    rows = _trace_event_rows()
    assert any(
        r.get("record_type") == "span_start" and r.get("name") == "decor_persist"
        for r in rows
    )


def test_manual_span_without_recorded_wrapper_does_not_persist():
    """Low-level ``Span`` is stack-only; integrators should use ``probing.span``."""
    from probing.tracing import Span

    parent = Span("manual_parent")
    child = Span.new_child(parent, "manual_child")
    child.end()
    rows = _trace_event_rows()
    assert rows == []


def test_status_and_duration():
    with probing.span("timed") as s:
        time.sleep(0.05)
    assert s.status == "Completed"
    assert s.is_ended
    assert s.duration is not None
    assert s.duration >= 0.05


def test_repr_contains_core_fields():
    with probing.span("repr_test") as s:
        r = repr(s)
        assert "Span" in r
        assert "repr_test" in r
        assert ("Active" in r) or ("Completed" in r)


def test_nested_decorator_and_context_manager():
    @probing.span("outer")
    def outer():
        with probing.span("inner") as inner:
            assert inner.name == "inner"
            return "ok"

    assert outer() == "ok"


def test_manual_construction_and_child():
    from probing.tracing import Span

    parent = Span("manual_parent")
    child = Span.new_child(parent, "manual_child")
    assert child.parent_id == parent.span_id
    assert child.trace_id == parent.trace_id
    child.end()
    assert child.is_ended


def test_add_event_module_function():
    """Test add_event module-level function."""
    with probing.span("test_add_event") as s:
        span_id = s.span_id
        probing.event("event1")
        probing.event("event2", attributes=[{"key": "value"}])

        events = s.get_events()
        assert len(events) == 2
        assert events[0]["name"] == "event1"
        assert events[1]["name"] == "event2"
        assert events[1]["attributes"]["key"] == "value"

    rows = _trace_event_rows()
    persisted = [
        r
        for r in rows
        if r.get("record_type") == "event" and r.get("span_id") == span_id
    ]
    assert len(persisted) == 2
    assert {r["name"] for r in persisted} == {"event1", "event2"}


def test_access_nonexistent_attribute_raises():
    with probing.span("attr") as s:
        with pytest.raises(AttributeError):
            _ = s.not_exist_field


# Ensure add_attr isn't exposed (immutability guarantee)
def test_no_add_attr_method():
    with probing.span("no_add") as s:
        assert not hasattr(s, "add_attr")
    assert not hasattr(s, "add_attr")


def test_add_event_no_active_span():
    """Test add_event raises error when no active span."""
    from probing.tracing import current_span

    # Ensure no active span
    assert current_span() is None

    with pytest.raises(RuntimeError, match="No active span"):
        probing.event("should_fail")


def test_phase_inferred_from_name():
    from probing.tracing.phases import BACKWARD, FORWARD, OPTIMIZER

    with probing.span("forward") as s:
        assert s.phase == FORWARD
    with probing.span("step") as s:
        assert s.phase == OPTIMIZER
    with probing.span("custom.op") as s:
        assert not s.phase


def test_explicit_phase_on_span():
    from probing.tracing.phases import BACKWARD

    with probing.span("compute", phase=BACKWARD) as s:
        assert s.phase == BACKWARD


def test_inferred_phase_persists_to_trace_event():
    with probing.span("forward"):
        pass
    rows = _trace_event_rows()
    start = next(r for r in rows if r.get("record_type") == "span_start")
    assert start["phase"] == "forward"


def test_record_span_without_training_phase():
    probing.record_span("train.step", duration_ns=1_000_000)
    rows = _trace_event_rows()
    start = next(
        r
        for r in rows
        if r.get("record_type") == "span_start" and r.get("name") == "train.step"
    )
    assert start["phase"] in ("", None) or not start["phase"]


def test_event_inside_training_phases_with_nested_spans():
    with probing.span("optimizer", phase="optimizer"):
        probing.event("batch.stats", attributes=[{"i": 0}, {"loss": 1.0}])


def test_optimizer_span_reentrant_with_torch_probe():
    from probing.profiling.torch_probe import TorchProbe, TorchProbeConfig
    from probing.tracing.phases import OPTIMIZER

    tracer = TorchProbe(config=TorchProbeConfig(enabled=True))
    with probing.span("outer", phase=OPTIMIZER) as outer:
        tracer._begin_train_step_span()
        assert not outer.is_ended
        tracer._end_train_step_span()
        assert not outer.is_ended
        probing.event("still.open")
    assert outer.is_ended


def test_post_step_hook_does_not_reset_local_step_across_batches():
    from probing.profiling.torch_probe import TorchProbe, TorchProbeConfig
    from probing.tracing.phases import OPTIMIZER

    probing.step(0)
    tracer = TorchProbe(config=TorchProbeConfig(enabled=True))
    tracer.finalized = True

    class FakeOpt:
        pass

    opt = FakeOpt()

    for expected in range(1, 6):
        with probing.span("step", phase=OPTIMIZER):
            tracer.post_step_hook(opt, (), {})
        assert probing.step.micro_step == expected

import time

import pytest

import probing


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
    with probing.span("immutable", kind="op") as s:
        original_id = s.span_id
        with pytest.raises(AttributeError):
            s.name = "changed"  # should not allow reassignment
        with pytest.raises(AttributeError):
            s.kind = "other"
        with pytest.raises(AttributeError):
            s.span_id = 123
        # ensure values unchanged
        assert s.span_id == original_id
        assert s.name == "immutable"
        assert s.kind == "op"


def test_events_recording():
    with probing.span("events") as s:
        s.add_event("e1")
        s.add_event("e2", attributes=[{"k": "v"}])
        events = s.get_events()
        assert len(events) == 2
        assert events[0]["name"] == "e1"
        assert events[1]["name"] == "e2"
        assert events[1]["attributes"]["k"] == "v"


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


def test_access_nonexistent_attribute_raises():
    with probing.span("attr") as s:
        with pytest.raises(AttributeError):
            _ = s.not_exist_field


# Ensure add_attr isn't exposed (immutability guarantee)
def test_no_add_attr_method():
    with probing.span("no_add") as s:
        assert not hasattr(s, "add_attr")
    assert not hasattr(s, "add_attr")


def test_add_event_module_function():
    """Test add_event module-level function."""
    with probing.span("test_add_event") as s:
        probing.event("event1")
        probing.event("event2", attributes=[{"key": "value"}])

        events = s.get_events()
        assert len(events) == 2
        assert events[0]["name"] == "event1"
        assert events[1]["name"] == "event2"
        assert events[1]["attributes"]["key"] == "value"


def test_add_event_no_active_span():
    """Test add_event raises error when no active span."""
    from probing.tracing import current_span

    # Ensure no active span
    assert current_span() is None

    with pytest.raises(RuntimeError, match="No active span"):
        probing.event("should_fail")


def test_event_inside_train_step_with_nested_spans():
    """Regression: batch train.step + nested spans + event (imagenet pattern)."""
    from probing.tracing import TRAIN_STEP_KIND

    with probing.span("batch", kind=TRAIN_STEP_KIND):
        with probing.span("forward", kind="nn.forward"):
            pass
        with probing.span("loss", kind="compute"):
            pass
        with probing.span("backward", kind="nn.backward"):
            pass
        with probing.span("step", kind="optim.step"):
            pass
        probing.event(
            "batch.stats",
            attributes=[{"i": 0}, {"loss": 1.0}],
        )


def test_train_step_reentrant_torch_probe_does_not_close_manual_span():
    """TorchProbe reentrant train.step must not end the outer span on step hook."""
    from probing.profiling.torch_probe import TorchProbe, TorchProbeConfig
    from probing.tracing import TRAIN_STEP_KIND

    tracer = TorchProbe(config=TorchProbeConfig(enabled=True))
    with probing.span("batch", kind=TRAIN_STEP_KIND) as outer:
        tracer._begin_train_step_span()
        assert not outer.is_ended
        tracer._end_train_step_span()
        assert not outer.is_ended
        probing.event("still.open")
    assert outer.is_ended


def test_post_step_hook_does_not_reset_local_step_across_batches():
    """Regression: stale curr_step must not sync_local_step back to 0/1."""
    from probing.profiling.torch_probe import TorchProbe, TorchProbeConfig
    from probing.tracing import TRAIN_STEP_KIND, current_local_step, sync_local_step

    sync_local_step(0)
    tracer = TorchProbe(config=TorchProbeConfig(enabled=True))
    tracer.finalized = True

    class FakeOpt:
        pass

    opt = FakeOpt()

    for expected in range(1, 6):
        with probing.span("batch", kind=TRAIN_STEP_KIND):
            tracer.post_step_hook(opt, (), {})
        assert current_local_step() == expected

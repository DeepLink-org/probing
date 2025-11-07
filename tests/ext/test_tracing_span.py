import time
import pytest
from probing.tracing import span, Span, current_span


def test_context_manager_basic():
    with span("root") as s:
        assert s.name == "root"
        assert s.status == "Active"
        assert not s.is_ended
    assert s.status == "Completed"
    assert s.is_ended


def test_decorator_named_and_plain():
    @span("decor_named")
    def f1():
        return 1

    @span  # implicit name from function
    def f2():
        return 2

    assert f1() == 1
    assert f2() == 2


def test_nested_parent_child_ids():
    with span("parent") as parent:
        assert parent.parent_id is None
        with span("child") as child:
            assert child.parent_id == parent.span_id
            assert child.trace_id == parent.trace_id
            with span("grandchild") as grandchild:
                assert grandchild.parent_id == child.span_id
                assert grandchild.trace_id == parent.trace_id


def test_current_span_stack_behavior():
    assert current_span() is None
    with span("a") as a:
        top = current_span()
        assert top is not None
        assert top.span_id == a.span_id
        with span("b") as b:
            top2 = current_span()
            assert top2.span_id == b.span_id
        # after inner exits
        again = current_span()
        assert again.span_id == a.span_id
    assert current_span() is None


def test_property_immutability():
    with span("immutable", kind="op") as s:
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
    with span("events") as s:
        s.add_event("e1")
        s.add_event("e2", attributes=[{"k": "v"}])
        events = s.get_events()
        assert len(events) == 2
        assert events[0]["name"] == "e1"
        assert events[1]["name"] == "e2"
        assert events[1]["attributes"]["k"] == "v"


def test_status_and_duration():
    with span("timed") as s:
        time.sleep(0.05)
    assert s.status == "Completed"
    assert s.is_ended
    assert s.duration is not None
    assert s.duration >= 0.05


def test_repr_contains_core_fields():
    with span("repr_test") as s:
        r = repr(s)
        assert "Span" in r
        assert "repr_test" in r
        assert ("Active" in r) or ("Completed" in r)


def test_nested_decorator_and_context_manager():
    @span("outer")
    def outer():
        with span("inner") as inner:
            assert inner.name == "inner"
            return "ok"
    assert outer() == "ok"


def test_manual_construction_and_child():
    parent = Span("manual_parent")
    child = Span.new_child(parent, "manual_child")
    assert child.parent_id == parent.span_id
    assert child.trace_id == parent.trace_id
    child.end()
    assert child.is_ended


def test_access_nonexistent_attribute_raises():
    with span("attr") as s:
        with pytest.raises(AttributeError):
            _ = s.not_exist_field


# Ensure add_attr isn't exposed (immutability guarantee)
def test_no_add_attr_method():
    with span("no_add") as s:
        assert not hasattr(s, "add_attr")
    assert not hasattr(s, "add_attr")


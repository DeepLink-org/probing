import pytest

import probing
from probing.tracing import (
    TRAIN_STEP_KIND,
    advance_local_step,
    set_step_bucket_size,
    step_snapshot,
    sync_local_step,
)


@pytest.fixture(autouse=True)
def reset_step_context():
    sync_local_step(0)
    set_step_bucket_size(1)
    yield
    sync_local_step(0)
    set_step_bucket_size(1)


def test_step_snapshot_and_bucket():
    set_step_bucket_size(10)
    sync_local_step(0)
    snap = step_snapshot()
    assert snap.local_step == 0
    assert snap.global_step == 0
    assert snap.bucket_size == 10

    sync_local_step(15)
    snap = step_snapshot()
    assert snap.local_step == 15
    assert snap.global_step == 1


def test_advance_local_step():
    sync_local_step(0)
    snap = advance_local_step()
    assert snap.local_step == 1
    assert snap.global_step == 1


def test_train_step_span_injects_coordinates():
    with probing.span("batch", kind=TRAIN_STEP_KIND) as s:
        assert s.kind == TRAIN_STEP_KIND
        attrs = dict(s.get_attributes())
        assert attrs["local_step"] == 0
        assert attrs["global_step"] == 0
        assert attrs["source"] == "manual"


def test_nested_train_step_is_reentrant():
    sync_local_step(3)
    with probing.span("outer", kind=TRAIN_STEP_KIND) as outer:
        with probing.span("inner", kind=TRAIN_STEP_KIND) as inner:
            assert inner.span_id == outer.span_id
    assert step_snapshot().local_step == 4

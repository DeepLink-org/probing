import pytest

import probing
from probing.tracing.phases import OPTIMIZER, reset_phase


@pytest.fixture(autouse=True)
def reset_step_context():
    reset_phase()
    probing.step(0)
    probing.step(micro_batches=1)
    yield
    probing.step(0)
    probing.step(micro_batches=1)


def test_step_snapshot_and_micro_batches():
    probing.step(0)
    probing.step(micro_batches=10)
    snap = probing.step.snapshot()
    assert snap.micro_step == 0
    assert snap.local_step == 0
    assert snap.global_step == 0
    assert snap.micro_batches == 10

    probing.step(15)
    assert probing.step.micro_step == 15
    assert probing.step.local_step == 1
    assert probing.step.global_step == 1


def test_advance_micro_step():
    probing.step(0)
    probing.step()
    assert probing.step.micro_step == 1
    assert probing.step.local_step == 1
    assert probing.step.global_step == 1


def test_optimizer_span_injects_coordinates():
    with probing.span("step", phase=OPTIMIZER) as s:
        assert s.phase == OPTIMIZER
        attrs = dict(s.get_attributes())
        assert attrs["micro_step"] == 0
        assert attrs["local_step"] == 0
        assert attrs["global_step"] == 0
        assert attrs["source"] == "manual"


def test_nested_optimizer_span_is_reentrant():
    probing.step(3)
    with probing.span("outer", phase=OPTIMIZER) as outer:
        with probing.span("inner", phase=OPTIMIZER) as inner:
            assert inner.span_id == outer.span_id
    assert probing.step.micro_step == 4
    assert probing.step.local_step == 4
    assert probing.step.global_step == 4

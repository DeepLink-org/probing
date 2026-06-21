from probing.tracing import phases


def test_infer_training_names():
    assert phases.infer("forward") == phases.FORWARD
    assert phases.infer("backward") == phases.BACKWARD
    assert phases.infer("step") == phases.OPTIMIZER


def test_resolve_explicit_phase():
    assert phases.resolve("iter", phases.FORWARD) == phases.FORWARD


def test_infer_from_stage():
    assert phases.infer_from_stage("pre forward") == phases.FORWARD
    assert phases.infer_from_stage("post backward") == phases.BACKWARD
    assert phases.infer_from_stage("pre step") == phases.OPTIMIZER
    assert phases.infer_from_stage("pre init") is None


def test_invalid_phase_raises():
    import pytest

    with pytest.raises(ValueError, match="invalid training phase"):
        phases.resolve("x", "custom")


def test_resolve_span_phase_only():
    name, phase = phases.resolve_span(None, phases.FORWARD)
    assert name == phases.FORWARD
    assert phase == phases.FORWARD


def test_resolve_span_name_only():
    name, phase = phases.resolve_span("forward", None)
    assert name == "forward"
    assert phase == phases.FORWARD


def test_resolve_span_explicit_display_name():
    name, phase = phases.resolve_span("compute", phases.BACKWARD)
    assert name == "compute"
    assert phase == phases.BACKWARD


def test_resolve_span_requires_one():
    import pytest

    with pytest.raises(TypeError, match="requires name and/or phase"):
        phases.resolve_span(None, None)

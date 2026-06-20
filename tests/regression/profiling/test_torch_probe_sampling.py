"""TorchProbe sampling: anchor pre/post pairing and step cleanup."""

from probing.profiling.torch_probe import TorchProbe, TorchProbeConfig


def _reset_hook_offset(tracer: TorchProbe) -> None:
    tracer._module_call_offset = 0
    tracer._current_module = None
    tracer._current_stage = None


def _bump_hook_offset(tracer: TorchProbe, mod, stage: str) -> None:
    tracer.process_hook(mod, stage)


class _FakeMod:
    pass


def _ready_tracer(curr_mod=None):
    tracer = TorchProbe(config=TorchProbeConfig(enabled=True))
    tracer.finalized = True
    tracer.sampled_step = True
    tracer.mod_names = {}
    root = _FakeMod()
    other = _FakeMod()
    tracer.mod_names[id(root)] = "model"
    tracer.mod_names[id(other)] = "model.features.conv"
    tracer.curr_mod = curr_mod if curr_mod is not None else id(other)
    tracer._open_spans = {}
    tracer.pending = []
    tracer.events = {}
    tracer.cpu_start = {}
    return tracer, root, other


def test_anchor_pre_always_pairs_post_when_curr_mod_differs():
    tracer, root, other = _ready_tracer()
    tracer.curr_mod = id(other)
    _reset_hook_offset(tracer)

    tracer.log_module_stage("pre forward", root)
    assert (id(root), "forward") in tracer._open_spans
    assert len(tracer.pending) == 1

    _bump_hook_offset(tracer, root, "pre forward")
    tracer.log_module_stage("post forward", root)
    assert (id(root), "forward") not in tracer._open_spans
    assert len(tracer.pending) == 2
    assert (
        tracer.pending[-1].events is not None
        or tracer.pending[-1].record.stage == "post forward"
    )


def test_finish_open_stages_closes_orphan_pre():
    tracer, root, _ = _ready_tracer()
    _reset_hook_offset(tracer)

    tracer.log_module_stage("pre forward", root)
    assert tracer._open_spans

    tracer._finish_open_stages()
    assert not tracer._open_spans
    assert any(r.record.stage == "post forward" for r in tracer.pending)


def test_cleanup_step_resources_clears_timers():
    tracer, root, _ = _ready_tracer()
    _reset_hook_offset(tracer)
    key = (id(root), "forward")

    tracer.cpu_start[key] = 1.0
    tracer.events[key] = object()

    tracer._cleanup_step_resources()
    assert not tracer._open_spans
    assert not tracer.events
    assert not tracer.cpu_start


def test_should_sample_anchor_only_on_pre_at_offset_zero():
    tracer, root, other = _ready_tracer()
    tracer.curr_mod = id(other)
    _reset_hook_offset(tracer)

    assert tracer.should_sample(root, "pre forward")
    assert not tracer.should_sample(root, "post forward")

    _bump_hook_offset(tracer, root, "pre forward")
    assert not tracer.should_sample(root, "pre forward")
    assert tracer.should_sample(other, "pre forward")

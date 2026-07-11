"""Integration-style checks for torch_step_timing shadow cadence."""

from probing.profiling.torch_probe import (
    TorchProbe,
    TorchProbeConfig,
    shadow_step_in_cycle,
)


class _FakeMod:
    pass


def _make_tracer(*, rate: float = 1.0) -> TorchProbe:
    tracer = TorchProbe(TorchProbeConfig(enabled=True, rate=rate))
    tracer.finalized = True
    tracer.sampled_step = True
    tracer.mod_names = {}
    root = _FakeMod()
    tracer.mod_names[id(root)] = "model"
    tracer._open_spans = {}
    tracer.pending = []
    tracer.events = {}
    tracer.cpu_start = {}
    tracer.has_backend = False
    tracer._step_cycle = 0
    tracer._refresh_shadow_flag()
    tracer._mark_step_wall_start()
    return tracer, root


def test_shadow_cadence_writes_alternating_rows(monkeypatch):
    saved = []

    def _capture_save(self):
        saved.append((self.is_shadow, self.sampled, self.sample_rate, self.sample_mode))

    monkeypatch.setattr(
        "probing.profiling.torch_probe.TorchStepTiming.save",
        _capture_save,
    )

    tracer, root = _make_tracer(rate=1.0)

    # One discovery-equivalent bootstrap: mark finalized already, simulate 6 steps.
    for _ in range(6):
        if not tracer.shadow_step:
            tracer.log_module_stage("pre forward", root)
            tracer.log_module_stage("post forward", root)
        tracer.post_step_hook(None, (), {})

    assert len(saved) == 6
    assert [row[0] for row in saved] == [
        0,
        0,
        0,
        0,
        1,
        0,
    ]
    assert saved[0][2] == 1.0
    assert saved[0][3] == "random"
    assert shadow_step_in_cycle(4) is True


def test_sample_rate_recorded_from_config(monkeypatch):
    saved = []

    def _capture_save(self):
        saved.append(self.sample_rate)

    monkeypatch.setattr(
        "probing.profiling.torch_probe.TorchStepTiming.save",
        _capture_save,
    )

    tracer, root = _make_tracer(rate=0.05)
    tracer.log_module_stage("pre forward", root)
    tracer.log_module_stage("post forward", root)
    tracer.post_step_hook(None, (), {})

    assert len(saved) == 1
    assert saved[0] == 0.05

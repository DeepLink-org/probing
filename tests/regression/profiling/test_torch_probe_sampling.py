"""TorchProbe sampling: anchor pre/post pairing and step cleanup."""

import time

from probing.profiling.torch_probe import (
    DEFAULT_SAMPLE_RATE,
    DEFAULT_SHADOW_BASELINE,
    DEFAULT_SHADOW_NORMAL,
    TorchProbe,
    TorchProbeConfig,
    configure,
    shadow_step_in_cycle,
)


def _reset_hook_offset(tracer: TorchProbe) -> None:
    tracer._module_call_offset = 0
    tracer._current_module = None
    tracer._current_stage = None


def _bump_hook_offset(tracer: TorchProbe, mod, stage: str) -> None:
    tracer.process_hook(mod, stage)


class _FakeMod:
    pass


def _ready_tracer(layer_rate: float = 1.0):
    tracer = TorchProbe(config=TorchProbeConfig(enabled=True))
    tracer.finalized = True
    tracer.sampled_step = True
    tracer.layer_rate = layer_rate
    tracer.mod_names = {}
    root = _FakeMod()
    other = _FakeMod()
    tracer.mod_names[id(root)] = "model"
    tracer.mod_names[id(other)] = "model.features.conv"
    tracer._open_spans = {}
    tracer.pending = []
    tracer.events = {}
    tracer.cpu_start = {}
    return tracer, root, other


def test_configure_works_without_torch_installed():
    """SET probing.torch.profiling=on must succeed before torch is imported."""
    cfg = configure("on")
    assert cfg.enabled
    assert cfg.rate == DEFAULT_SAMPLE_RATE


def test_default_sample_rate_is_five_percent():
    assert TorchProbeConfig.parse("on").rate == DEFAULT_SAMPLE_RATE
    assert DEFAULT_SAMPLE_RATE == 0.05


def test_default_shadow_cadence_is_four_to_one():
    cfg = TorchProbeConfig.parse("on")
    assert cfg.shadow_normal == DEFAULT_SHADOW_NORMAL == 4
    assert cfg.shadow_baseline == DEFAULT_SHADOW_BASELINE == 1
    assert cfg.shadow_enabled


def test_shadow_spec_parse_and_disable():
    cfg = TorchProbeConfig.parse("on,shadow=8:2")
    assert cfg.shadow_normal == 8
    assert cfg.shadow_baseline == 2
    cfg.apply_shadow_spec("off")
    assert not cfg.shadow_enabled


def test_shadow_step_in_cycle_pattern():
    assert [shadow_step_in_cycle(i) for i in range(10)] == [
        False,
        False,
        False,
        False,
        True,
        False,
        False,
        False,
        False,
        True,
    ]


def test_shadow_step_skips_forward_hooks():
    tracer, root, _ = _ready_tracer()
    tracer.shadow_step = True
    tracer.pre_forward_hook(root, None)
    tracer.post_forward_hook(root, None, None)
    assert tracer._module_call_offset == 0
    assert not tracer._open_spans
    assert not tracer.pending


def test_record_step_timing_writes_shadow_and_probed(monkeypatch):
    saved = []

    def _capture_save(self):
        saved.append(
            (
                self.is_shadow,
                self.sampled,
                self.step_duration_sec,
                self.shadow_normal,
                self.shadow_baseline,
            )
        )

    monkeypatch.setattr(
        "probing.profiling.torch_probe.TorchStepTiming.save",
        _capture_save,
    )

    tracer, _, _ = _ready_tracer()
    tracer._mark_step_wall_start()
    time.sleep(0.01)
    tracer._record_step_timing(is_shadow=False)
    tracer._mark_step_wall_start()
    time.sleep(0.01)
    tracer._record_step_timing(is_shadow=True)

    assert len(saved) == 2
    assert saved[0][0] == 0
    assert saved[0][1] == 1
    assert saved[0][2] > 0
    assert saved[1][0] == 1
    assert saved[1][1] == 0
    assert saved[1][2] > 0


def test_post_step_hook_records_probed_timing_after_flush(monkeypatch):
    order = []
    orig_finish = TorchProbe._finish_open_stages

    def track_finish(self):
        order.append("finish")
        orig_finish(self)

    def track_timing(self, *, is_shadow: bool):
        order.append(("timing", is_shadow))

    monkeypatch.setattr(TorchProbe, "_finish_open_stages", track_finish)
    monkeypatch.setattr(TorchProbe, "_record_step_timing", track_timing)

    tracer, root, _ = _ready_tracer()
    tracer.shadow_step = False
    tracer.has_backend = False
    tracer._mark_step_wall_start()

    tracer.log_module_stage("pre forward", root)
    tracer.log_module_stage("post forward", root)

    tracer.post_step_hook(None, (), {})

    assert "finish" in order
    timing_events = [e for e in order if isinstance(e, tuple) and e[0] == "timing"]
    assert timing_events == [("timing", False)]
    # Open stages are flushed before the step's wall timing is recorded.
    assert order.index("finish") < order.index(timing_events[0])


def test_post_step_hook_drains_deferred_after_step_timing(monkeypatch):
    order = []

    def track_timing(self, *, is_shadow: bool):
        order.append("timing")

    def track_drain(self):
        order.append("drain")

    monkeypatch.setattr(TorchProbe, "_record_step_timing", track_timing)
    monkeypatch.setattr(TorchProbe, "_drain_deferred", track_drain)
    monkeypatch.setattr(
        TorchProbe, "_advance_step_cycle_for_next", lambda self: order.append("advance")
    )
    monkeypatch.setattr(
        TorchProbe, "_mark_step_wall_start", lambda self: order.append("mark")
    )

    tracer, _, _ = _ready_tracer()
    tracer.shadow_step = False
    tracer.has_backend = False
    tracer._mark_step_wall_start()

    tracer.post_step_hook(None, (), {})

    assert order[-4:] == ["timing", "drain", "advance", "mark"]


def test_anchor_pre_always_pairs_post_even_when_layer_not_sampled():
    # layer_rate=0 → nothing sampled except the offset-0 time anchor.
    tracer, root, other = _ready_tracer(layer_rate=0.0)
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


def test_backward_config_parse_default_off():
    cfg = TorchProbeConfig.parse("on")
    assert not cfg.backward
    assert not TorchProbeConfig.parse("on,backward=off").backward
    assert TorchProbeConfig.parse("on,backward=on").backward
    assert TorchProbeConfig.parse("random:1.0,backward=true").backward


def test_cpu_fallback_duration_is_positive_seconds():
    """CPU/MPS timing must not mix time.time() with perf_counter()."""
    import torch
    import torch.nn as nn

    tracer = TorchProbe(config=TorchProbeConfig(enabled=True))
    tracer.finalized = True
    tracer.sampled_step = True
    tracer.use_gpu_events = False
    tracer.has_backend = False
    linear = nn.Linear(4, 2)
    tracer.mod_names = {id(linear): "linear"}
    tracer._open_spans = {}
    tracer.pending = []
    tracer.events = {}
    tracer.cpu_start = {}

    x = torch.randn(1, 4)
    tracer.pre_forward_hook(linear, (x,))
    out = linear(x)
    tracer.post_forward_hook(linear, (x,), out)
    post = next(p for p in tracer.pending if p.record.stage == "post forward")
    post.save()
    assert 0.0 < post.record.duration < 1.0


def test_optimizer_step_recorded_every_sampled_step():
    import torch
    import torch.nn as nn

    from probing.profiling.types import BaseTracer

    tracer = TorchProbe(config=TorchProbeConfig(enabled=True))
    tracer.finalized = True
    tracer.sampled_step = True
    tracer.rate = 1.0
    tracer.use_gpu_events = False
    tracer.has_backend = False
    model = nn.Linear(4, 2)
    opt = torch.optim.SGD(model.parameters(), lr=0.01)
    tracer.mod_names = {id(model): "model", id(opt): "SGD"}

    for _ in range(3):
        tracer._open_spans = {}
        tracer.pending = []
        tracer.events = {}
        tracer.cpu_start = {}
        tracer._module_call_offset = 0
        x = torch.randn(1, 4)
        tracer.pre_forward_hook(model, (x,))
        tracer.post_forward_hook(model, (x,), model(x))
        BaseTracer.pre_step_hook(tracer, opt, (), {})
        BaseTracer.post_step_hook(tracer, opt, (), {})
        stages = {p.record.stage for p in tracer.pending}
        assert "post step" in stages


def test_sampled_step_defers_gpu_event_reads(monkeypatch):
    """Async-backend sampled steps defer GPU timing reads until events complete.

    Reading ``elapsed_time`` needs completed events, which would force a blocking
    ``synchronize`` on the sampled step (the stall that inflates its wall time).
    Records are instead stashed and drained on later optimizer steps once
    ``event.query()`` reports the GPU has caught up — so the sampled step never
    blocks.
    """
    import torch
    import torch.nn as nn

    from probing.profiling import torch_probe as tp
    from probing.profiling.types import BaseTracer

    class FakeEvent:
        ready = False  # class-wide gate simulating GPU completion

        def record(self):
            pass

        def query(self):
            return FakeEvent.ready

        def elapsed_time(self, _other):
            return 2.5

    monkeypatch.setattr(tp, "_backend_event", lambda enable_timing=True: FakeEvent())

    tracer = TorchProbe(config=TorchProbeConfig(enabled=True))
    tracer.finalized = True
    tracer.sampled_step = True
    tracer.rate = 1.0
    tracer.use_gpu_events = True
    tracer.has_backend = True
    model = nn.Linear(4, 2)
    opt = torch.optim.SGD(model.parameters(), lr=0.01)
    tracer.mod_names = {id(model): "model", id(opt): "SGD"}

    def run_step():
        tracer._open_spans = {}
        tracer.pending = []
        tracer.events = {}
        tracer.cpu_start = {}
        tracer._module_call_offset = 0
        x = torch.randn(1, 4)
        tracer.pre_forward_hook(model, (x,))
        tracer.post_forward_hook(model, (x,), model(x))
        BaseTracer.pre_step_hook(tracer, opt, (), {})
        tracer.post_step_hook(opt, (), {})

    # GPU busy: records deferred off the sampled step, nothing left in pending.
    FakeEvent.ready = False
    run_step()
    assert tracer._deferred, "expected GPU-timed records to be deferred"
    assert tracer.pending == []
    deferred_have_events = [d for d in tracer._deferred if d.events is not None]
    assert deferred_have_events, "deferred records should retain their GPU events"

    # Still busy next step → still deferred (no blocking read).
    run_step()
    assert tracer._deferred

    # GPU caught up → drained on subsequent optimizer steps, but only after the
    # records have aged past the settle window (so their reads are free).
    FakeEvent.ready = True
    tracer._drain_deferred()
    assert tracer._deferred, "records must settle a few steps before being read"
    tracer._step_cycle += tp._DEFER_MIN_SETTLE
    tracer._drain_deferred()
    assert tracer._deferred == []


def test_deferred_reads_force_flush_when_stale(monkeypatch):
    """A record deferred longer than ``_DEFER_MAX_LAG`` is force-completed."""
    import torch
    import torch.nn as nn

    from probing.profiling import torch_probe as tp
    from probing.profiling.types import BaseTracer

    synced = {"n": 0}

    class FakeEvent:
        def record(self):
            pass

        def query(self):
            return False  # never ready on its own

        def synchronize(self):
            synced["n"] += 1

        def elapsed_time(self, _other):
            return 1.0

    monkeypatch.setattr(tp, "_backend_event", lambda enable_timing=True: FakeEvent())

    tracer = TorchProbe(config=TorchProbeConfig(enabled=True))
    tracer.finalized = True
    tracer.sampled_step = True
    tracer.rate = 1.0
    tracer.use_gpu_events = True
    tracer.has_backend = True
    model = nn.Linear(4, 2)
    opt = torch.optim.SGD(model.parameters(), lr=0.01)
    tracer.mod_names = {id(model): "model", id(opt): "SGD"}

    tracer._open_spans = {}
    tracer.pending = []
    tracer.events = {}
    tracer.cpu_start = {}
    tracer._module_call_offset = 0
    x = torch.randn(1, 4)
    tracer.pre_forward_hook(model, (x,))
    tracer.post_forward_hook(model, (x,), model(x))
    BaseTracer.pre_step_hook(tracer, opt, (), {})
    tracer.post_step_hook(opt, (), {})
    assert tracer._deferred

    # Advance the cycle well past the lag budget; drain must force-complete.
    tracer._step_cycle += tp._DEFER_MAX_LAG + 1
    tracer._drain_deferred()
    assert tracer._deferred == []
    assert synced["n"] > 0, "stale deferred events should be force-synchronized"


def test_deferred_reads_wait_for_settle_window(monkeypatch):
    """A GPU-event timing read must not happen until the record has settled.

    Regression guard for the sampled-step overhead: even when ``event.query()``
    reports ready, reading ``elapsed_time`` on the very next optimizer step (while
    the kernels the events bracket may still be draining on the GPU) forces a
    blocking commit — tens of ms per sampled step, worst for backward events
    recorded near the end of a step. ``_drain_deferred`` must therefore hold a
    record for ``_DEFER_MIN_SETTLE`` optimizer steps before it reads its timing,
    so the read is a free cached lookup. This test fails if that settle window is
    removed or ``elapsed_time`` is read prematurely.
    """
    import torch
    import torch.nn as nn

    from probing.profiling import torch_probe as tp
    from probing.profiling.types import BaseTracer

    reads = {"n": 0}

    class FakeEvent:
        def record(self):
            pass

        def query(self):
            return True  # GPU always reports ready → only the settle gate applies

        def elapsed_time(self, _other):
            reads["n"] += 1
            return 2.5

    monkeypatch.setattr(tp, "_backend_event", lambda enable_timing=True: FakeEvent())

    # The whole point of settling is to defer reads by a few steps.
    assert tp._DEFER_MIN_SETTLE >= 1
    assert tp._DEFER_MIN_SETTLE < tp._DEFER_MAX_LAG

    tracer = TorchProbe(config=TorchProbeConfig(enabled=True))
    tracer.finalized = True
    tracer.sampled_step = True
    tracer.rate = 1.0
    tracer.use_gpu_events = True
    tracer.has_backend = True
    model = nn.Linear(4, 2)
    opt = torch.optim.SGD(model.parameters(), lr=0.01)
    tracer.mod_names = {id(model): "model", id(opt): "SGD"}

    tracer._open_spans = {}
    tracer.pending = []
    tracer.events = {}
    tracer.cpu_start = {}
    tracer._module_call_offset = 0
    x = torch.randn(1, 4)
    tracer.pre_forward_hook(model, (x,))
    tracer.post_forward_hook(model, (x,), model(x))
    BaseTracer.pre_step_hook(tracer, opt, (), {})
    tracer.post_step_hook(opt, (), {})
    deferred = [d for d in tracer._deferred if d.events is not None]
    assert deferred, "sampled step should defer GPU-timed records"
    deferred_count = len(tracer._deferred)
    base = deferred[0]._defer_cycle

    # Drain repeatedly *within* the settle window: despite query()==True, no
    # timing read may occur and nothing may be persisted yet.
    for cycle in range(base, base + tp._DEFER_MIN_SETTLE):
        tracer._step_cycle = cycle
        tracer._drain_deferred()
        assert reads["n"] == 0, "elapsed_time read before the settle window elapsed"
        assert len(tracer._deferred) == deferred_count

    # One step past the settle window → reads happen and the queue drains.
    tracer._step_cycle = base + tp._DEFER_MIN_SETTLE
    tracer._drain_deferred()
    assert reads["n"] == len(deferred)
    assert tracer._deferred == []


def test_non_sampled_step_skips_hook_dispatch():
    """At low rates, most steps should not run module hooks or GPU flush."""
    import torch
    import torch.nn as nn

    tracer = TorchProbe(config=TorchProbeConfig(enabled=True))
    tracer.finalized = True
    tracer.sampled_step = False
    tracer.rate = 0.01
    tracer.use_gpu_events = False
    tracer.has_backend = False
    model = nn.Linear(4, 2)
    opt = torch.optim.SGD(model.parameters(), lr=0.01)
    tracer.mod_names = {id(model): "model", id(opt): "SGD"}
    tracer._open_spans = {}
    tracer.pending = []
    tracer.events = {}
    tracer.cpu_start = {}
    tracer._module_call_offset = 0

    x = torch.randn(1, 4)
    tracer.pre_forward_hook(model, (x,))
    tracer.post_forward_hook(model, (x,), model(x))
    assert tracer._module_call_offset == 0
    assert tracer.pending == []

    tracer.pre_step_hook(opt, (), {})
    assert tracer.pending == []

    tracer.post_step_hook(opt, (), {})
    assert tracer.pending == []


def _backward_saved(tracer):
    """Records captured by immediate ``post backward`` save (not left in pending)."""
    import dataclasses

    from probing.profiling.torch_probe import TorchTrace

    fields = [f.name for f in dataclasses.fields(TorchTrace)]
    return [dict(zip(fields, d)) for _, d in TorchTrace.take(200)]


def test_backward_respects_sampling():
    """Backward grad hooks mirror the modules whose forward was recorded."""
    import torch
    import torch.nn as nn

    tracer = TorchProbe(config=TorchProbeConfig(enabled=True, backward=True))
    tracer.finalized = True
    tracer.sampled_step = True
    tracer.rate = 1.0
    tracer.use_gpu_events = False
    tracer.has_backend = False
    linear = nn.Linear(4, 2)
    tracer.mod_names = {id(linear): "linear"}
    tracer._open_spans = {}
    tracer.pending = []
    tracer.events = {}
    tracer.cpu_start = {}
    tracer._sampled_mods_this_step = set()

    x = torch.randn(1, 4, requires_grad=True)
    # layer_rate=0 and offset > 0 (not the anchor): forward is not recorded,
    # so backward must not attach.
    tracer.layer_rate = 0.0
    tracer._module_call_offset = 1
    tracer.pre_forward_hook(linear, (x,))
    out = linear(x)
    tracer.post_forward_hook(linear, (x,), out)
    assert id(linear) not in tracer._sampled_mods_this_step
    assert (id(linear), "backward") not in tracer._open_spans

    # layer_rate=1 → forward recorded → backward grad hooks attach (the backward
    # span opens lazily when grad_output arrives, not at forward time).
    tracer.layer_rate = 1.0
    tracer._open_spans = {}
    tracer.pending = []
    tracer._module_call_offset = 1
    tracer.pre_forward_hook(linear, (x,))
    out2 = linear(x)
    tracer.post_forward_hook(linear, (x,), out2)
    assert id(linear) in tracer._sampled_mods_this_step

    out2.sum().backward()
    saved = _backward_saved(tracer)
    assert any(r.get("stage") == "post backward" for r in saved)


def test_backward_hooks_record_when_enabled():
    import torch
    import torch.nn as nn

    tracer = TorchProbe(config=TorchProbeConfig(enabled=True, backward=True))
    tracer.finalized = True
    tracer.sampled_step = True
    tracer.rate = 1.0
    tracer.mod_names = {}
    linear = nn.Linear(4, 2)
    tracer.mod_names[id(linear)] = "linear"
    tracer._open_spans = {}
    tracer.pending = []
    tracer.events = {}
    tracer.cpu_start = {}

    # Input requires grad so grad_input fires and the backward interval (grad_output
    # → grad_input) is measurable.
    x = torch.randn(1, 4, requires_grad=True)
    tracer.pre_forward_hook(linear, (x,))
    out = linear(x)
    tracer.post_forward_hook(linear, (x,), out)
    assert (
        id(linear),
        "backward",
    ) not in tracer._open_spans  # opens lazily at grad_output

    out.sum().backward()
    assert (id(linear), "backward") not in tracer._open_spans
    saved = _backward_saved(tracer)
    post = next(r for r in saved if r.get("stage") == "post backward")
    assert post.get("duration", 0) > 0


def test_backward_tensor_hooks_with_inplace_relu():
    import torch
    import torch.nn as nn

    tracer = TorchProbe(config=TorchProbeConfig(enabled=True, backward=True))
    tracer.finalized = True
    tracer.sampled_step = True
    tracer.rate = 1.0
    tracer.mod_names = {}
    conv = nn.Conv2d(3, 4, kernel_size=3, padding=1)
    relu = nn.ReLU(inplace=True)
    model = nn.Sequential(conv, relu)
    tracer.mod_names[id(conv)] = "conv"
    tracer._open_spans = {}
    tracer.pending = []
    tracer.events = {}
    tracer.cpu_start = {}

    x = torch.randn(1, 3, 8, 8, requires_grad=True)
    tracer.pre_forward_hook(conv, (x,))
    out = conv(x)
    tracer.post_forward_hook(conv, (x,), out)
    loss = relu(out).sum()
    loss.backward()
    saved = _backward_saved(tracer)
    assert any(r.get("stage") == "post backward" for r in saved)

    x2 = torch.randn(1, 3, 8, 8)
    tracer.pre_forward_hook(conv, (x2,))
    out2 = conv(x2)
    tracer.post_forward_hook(conv, (x2,), out2)
    relu(out2).sum().backward()


def test_backward_hooks_skipped_on_shadow_step():
    tracer, root, _ = _ready_tracer()
    tracer.config = TorchProbeConfig(enabled=True, backward=True)
    tracer.shadow_step = True
    import torch

    t = torch.randn(2, 2, requires_grad=True)
    tracer.post_forward_hook(root, (t,), t)
    assert not tracer.pending
    assert not tracer._open_spans


def test_install_hooks_backward_flag():
    import torch.nn as nn

    from probing.profiling import torch as torch_prof
    from probing.profiling.torch_probe import TorchProbe, TorchProbeConfig

    class M(nn.Module):
        def forward(self, x):
            return x

    model = M()
    tracer = TorchProbe(config=TorchProbeConfig(enabled=True, backward=True))
    try:
        torch_prof.install_hooks(model, tracer=tracer, backward=True)
        assert len(torch_prof.HOOK_CACHE[id(model)]) == 2
    finally:
        torch_prof.uninstall_hooks()


def test_should_sample_anchor_and_layer_rate():
    tracer, root, other = _ready_tracer(layer_rate=0.0)
    _reset_hook_offset(tracer)

    # Anchor: first pre hook at offset 0 is always sampled (pairs with its post).
    assert tracer.should_sample(root, "pre forward")
    assert not tracer.should_sample(root, "post forward")

    # Past the anchor with layer_rate=0 → nothing is sampled.
    _bump_hook_offset(tracer, root, "pre forward")
    assert not tracer.should_sample(root, "pre forward")
    assert not tracer.should_sample(other, "pre forward")

    # Full snapshot (layer_rate=1) → every layer past the anchor is sampled.
    tracer.layer_rate = 1.0
    assert tracer.should_sample(other, "pre forward")


def _plan_sequence(rate: float, cycles: int):
    """Deterministic per-cycle ``sampled_step`` plan for a fresh tracer."""
    tracer = TorchProbe(config=TorchProbeConfig(enabled=True, rate=rate))
    tracer.mod_names = {i: f"m{i}" for i in range(4)}
    tracer.finalize_discovery()  # enables auto-plan
    seq = []
    for c in range(cycles):
        tracer._step_cycle = c
        tracer._planned_cycle = None
        tracer._ensure_step_plan()
        seq.append(tracer.sampled_step)
    return seq


def test_sampling_plan_is_deterministic_and_rank_aligned():
    """P3: the plan is a pure function of the step index, so every rank agrees."""
    seq_a = _plan_sequence(0.25, 40)
    seq_b = _plan_sequence(0.25, 40)
    assert seq_a == seq_b
    # Stratified: rate 0.25 -> period 4 -> sample cycles 0, 4, 8, ...
    sampled_cycles = [c for c, s in enumerate(seq_a) if s]
    assert sampled_cycles == list(range(0, 40, 4))
    # The first probed step must be sampled (no long startup gap).
    assert seq_a[0] is True


def test_random_two_rate_spec_parses_step_and_layer_rate():
    """``random:step:layer`` sets step density and per-layer hit probability."""
    cfg = TorchProbeConfig.parse("random:0.1:0.3,backward=on")
    assert cfg.mode == "random"
    assert cfg.rate == 0.1
    assert cfg.layer_rate == 0.3
    assert cfg.backward
    # Single-rate random keeps full-snapshot default (layer_rate=1.0).
    assert TorchProbeConfig.parse("random:0.1").layer_rate == 1.0
    # layer_rate= key also works.
    assert TorchProbeConfig.parse("random:0.2,layer_rate=0.5").layer_rate == 0.5


def test_random_layer_selection_is_deterministic_subset():
    """On a sampled step, random mode hits a per-layer subset, reproducibly."""
    mods = [_FakeMod() for _ in range(200)]

    def hits(seed_cycle):
        tr = TorchProbe(config=TorchProbeConfig(enabled=True, mode="random", rate=1.0))
        tr.layer_rate = 0.5
        tr.mod_names = {id(m): f"layer{i}" for i, m in enumerate(mods)}
        tr.finalize_discovery()
        tr._step_cycle = seed_cycle
        tr._planned_cycle = None
        tr._ensure_step_plan()
        assert tr.sampled_step  # rate 1.0 -> every step sampled
        # offset > 0 so the anchor rule (first pre hook) does not apply.
        tr._module_call_offset = 1
        return {id(m) for m in mods if tr.should_sample(m, "pre forward")}

    a = hits(3)
    assert a == hits(3)  # deterministic / reproducible
    # ~50% of layers hit (loose bounds for hash noise over 200 layers).
    assert 60 <= len(a) <= 140
    # Different step -> different subset (not a fixed mask).
    assert hits(4) != a


def test_low_rate_samples_first_step_and_stays_on_cadence():
    """Low rates must not leave a long startup gap (regression: 0 samples)."""
    for rate, period in ((0.01, 100), (0.05, 20)):
        seq = _plan_sequence(rate, period * 3 + 1)
        sampled_cycles = [c for c, s in enumerate(seq) if s]
        # Immediate first sample + steady, evenly-spaced cadence.
        assert sampled_cycles[0] == 0
        assert sampled_cycles == list(range(0, period * 3 + 1, period))


def test_sampling_does_not_touch_global_rng():
    """P2: sampling decisions never perturb the host RNG stream."""
    import random as _random

    _random.seed(1234)
    before = _random.getstate()
    _plan_sequence(0.5, 100)
    _plan_sequence(0.1, 100)
    assert _random.getstate() == before


def test_random_mode_short_circuits_non_sampled_steps():
    """P1: ``random`` mode also skips hook dispatch on non-sampled steps."""
    tracer = TorchProbe(config=TorchProbeConfig(enabled=True, mode="random", rate=0.3))
    tracer.mod_names = {i: f"m{i}" for i in range(4)}
    tracer.finalize_discovery()

    seen_active = seen_idle = False
    for c in range(60):
        tracer._step_cycle = c
        tracer._planned_cycle = None
        active = tracer._hooks_dispatch_active()
        assert active == tracer.sampled_step
        seen_active |= active
        seen_idle |= not active
    # Non-sampled random steps exist and short-circuit (would previously always run).
    assert seen_active and seen_idle

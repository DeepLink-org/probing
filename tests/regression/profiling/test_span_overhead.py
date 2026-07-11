"""Regression: span emission overhead stays bounded when backends are enabled."""

from __future__ import annotations

import statistics
import time

import pytest

import probing
from probing.profiling.torch_probe import (
    DelayedRecord,
    TorchProbe,
    TorchProbeConfig,
    TorchStepTiming,
)
from probing.tracing import TraceEvent, bind_table, reset_backends
from probing.tracing.backends import configure as configure_backends
from probing.tracing.phases import FORWARD


class _FakeMod:
    pass


@pytest.fixture(autouse=True)
def _tracing_tables():
    reset_backends(clear_registered=True)
    probing.step(0)
    probing.step(micro_batches=1)
    try:
        TraceEvent.drop()
    except Exception:
        pass
    TraceEvent.init_table()
    bind_table(TraceEvent)
    yield
    reset_backends(clear_registered=True)


def _run_manual_spans(*, backends: list[str], iterations: int) -> float:
    configure_backends(backends)
    probing.step(0)
    started = time.perf_counter()
    for i in range(iterations):
        with probing.span(f"iter-{i}", phase=FORWARD, source="test"):
            pass
    return time.perf_counter() - started


def test_manual_span_memtable_overhead_bounded():
    """``probing.span`` with memtable backend should not dominate a tight loop."""
    iterations = 300
    off = _run_manual_spans(backends=[], iterations=iterations)
    on = _run_manual_spans(backends=["memtable"], iterations=iterations)

    # Guard against flaky CI: allow generous headroom but catch regressions
    # where span persistence becomes orders of magnitude slower.
    assert (
        on < off * 8.0 + 0.05
    ), f"memtable spans too slow: off={off:.4f}s on={on:.4f}s ratio={on / max(off, 1e-9):.2f}"


def _make_tracer(*, trace_spans: bool) -> tuple[TorchProbe, _FakeMod]:
    cfg = TorchProbeConfig.parse("on,shadow=off")
    cfg.trace_spans = trace_spans
    tracer = TorchProbe(config=cfg)
    tracer.finalized = True
    tracer.sampled_step = True
    tracer.has_backend = False
    root = _FakeMod()
    tracer.mod_names = {id(root): "model"}
    tracer._open_spans = {}
    tracer.pending = []
    tracer.events = {}
    tracer.cpu_start = {}
    tracer._step_cycle = 0
    tracer._refresh_shadow_flag()
    return tracer, root


def test_torch_probe_module_spans_overhead_bounded(monkeypatch):
    """TorchProbe per-module spans should add modest wall time vs hooks-only."""
    captured: list[float] = []

    def _capture_timing(self):
        captured.append(self.step_duration_sec)

    monkeypatch.setattr(TorchStepTiming, "save", _capture_timing)
    monkeypatch.setattr(DelayedRecord, "save", lambda self: None)

    configure_backends(["memtable"])

    tracer_off, root = _make_tracer(trace_spans=False)
    probing.step(0)
    for _ in range(20):
        tracer_off._mark_step_wall_start()
        tracer_off.log_module_stage("pre forward", root)
        tracer_off.log_module_stage("post forward", root)
        tracer_off.post_step_hook(None, (), {})
    off_samples = captured.copy()
    captured.clear()

    tracer_on, root = _make_tracer(trace_spans=True)
    probing.step(0)
    for _ in range(20):
        tracer_on._mark_step_wall_start()
        tracer_on.log_module_stage("pre forward", root)
        tracer_on.log_module_stage("post forward", root)
        tracer_on.post_step_hook(None, (), {})
    on_samples = captured

    assert len(off_samples) == 20
    assert len(on_samples) == 20

    med_off = statistics.median(off_samples)
    med_on = statistics.median(on_samples)
    assert med_on < med_off * 6.0 + 0.02, (
        f"module spans too slow: off={med_off:.6f}s on={med_on:.6f}s "
        f"ratio={med_on / max(med_off, 1e-9):.2f}"
    )


def test_trace_spans_config_parse():
    cfg = TorchProbeConfig.parse("on,trace_spans=off,shadow=off")
    assert cfg.enabled
    assert not cfg.trace_spans
    cfg2 = TorchProbeConfig.parse("on,spans=0")
    assert not cfg2.trace_spans

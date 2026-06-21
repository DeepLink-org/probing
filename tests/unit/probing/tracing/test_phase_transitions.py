"""Unit tests: span-driven and hook-driven training phase state transitions."""

from __future__ import annotations

import dataclasses

import pytest

import probing
from probing.tracing import TraceEvent, bind_table, reset_backends
from probing.tracing.phases import (
    BACKWARD,
    FORWARD,
    IDLE,
    OPTIMIZER,
    hook_enter,
    hook_exit,
    phase,
    reset_phase,
)


@pytest.fixture(autouse=True)
def _isolated_tracing():
    reset_phase()
    probing.step(0)
    probing.step(micro_batches=1)
    try:
        TraceEvent.drop()
    except Exception:
        pass
    TraceEvent.init_table()
    reset_backends(clear_registered=True)
    bind_table(TraceEvent)
    yield
    reset_backends(clear_registered=True)
    reset_phase()


def _trace_rows(n: int = 100) -> list[dict]:
    fields = [f.name for f in dataclasses.fields(TraceEvent)]
    return [dict(zip(fields, data)) for _ts, data in TraceEvent.take(n)]


def _closed_span_names(rows: list[dict]) -> list[str]:
    starts = {
        r["span_id"]: r["name"] for r in rows if r.get("record_type") == "span_start"
    }
    ends = {r["span_id"] for r in rows if r.get("record_type") == "span_end"}
    return [starts[sid] for sid in ends if sid in starts]


# --- Span-driven transitions ---


class TestSpanDrivenPhaseTransitions:
    def test_starts_idle(self):
        assert phase() == IDLE

    def test_single_forward_idle_cycle(self):
        with probing.span("forward", phase=FORWARD):
            assert phase() == FORWARD
        assert phase() == IDLE

    def test_full_training_step_sequence(self):
        assert phase() == IDLE
        with probing.span("forward", phase=FORWARD):
            assert phase() == FORWARD
        assert phase() == IDLE

        with probing.span("backward", phase=BACKWARD):
            assert phase() == BACKWARD
        assert phase() == IDLE

        with probing.span("step", phase=OPTIMIZER):
            assert phase() == OPTIMIZER
        assert phase() == IDLE
        assert probing.step.micro_step == 1

    def test_nested_training_phases_restore_parent(self):
        with probing.span("forward", phase=FORWARD):
            assert phase() == FORWARD
            with probing.span("backward", phase=BACKWARD):
                assert phase() == BACKWARD
            assert phase() == FORWARD
        assert phase() == IDLE

    def test_non_training_span_leaves_phase_idle(self):
        with probing.span("data.load"):
            assert phase() == IDLE
        with probing.span("epoch"):
            assert phase() == IDLE

    def test_inferred_phase_from_name(self):
        with probing.span("forward"):
            assert phase() == FORWARD
        assert phase() == IDLE

    def test_train_step_name_does_not_change_phase(self):
        probing.record_span("train.step", duration_ns=1000)
        assert phase() == IDLE

    def test_outer_batch_inner_forward(self):
        with probing.span("batch"):
            assert phase() == IDLE
            with probing.span("forward", phase=FORWARD):
                assert phase() == FORWARD
            assert phase() == IDLE
        assert phase() == IDLE

    def test_optimizer_reentrant_does_not_double_step(self):
        with probing.span("step", phase=OPTIMIZER):
            assert phase() == OPTIMIZER
            with probing.span("step", phase=OPTIMIZER):
                assert phase() == OPTIMIZER
        assert phase() == IDLE
        assert probing.step.micro_step == 1


# --- Hook-driven transitions ---


class TestHookDrivenPhaseTransitions:
    def test_hook_forward_cycle(self):
        hook_enter(FORWARD)
        assert phase() == FORWARD
        hook_exit(FORWARD)
        assert phase() == IDLE

    def test_hook_full_iteration_sequence(self):
        hook_enter(FORWARD)
        assert phase() == FORWARD
        hook_exit(FORWARD)
        assert phase() == IDLE

        hook_enter(BACKWARD)
        assert phase() == BACKWARD
        hook_exit(BACKWARD)
        assert phase() == IDLE

        hook_enter(OPTIMIZER)
        assert phase() == OPTIMIZER
        hook_exit(OPTIMIZER)
        assert phase() == IDLE

    def test_hook_emits_phase_spans(self):
        hook_enter(FORWARD)
        hook_exit(FORWARD)
        hook_enter(BACKWARD)
        hook_exit(BACKWARD)
        hook_enter(OPTIMIZER)
        hook_exit(OPTIMIZER)

        names = _closed_span_names(_trace_rows())
        assert names.count("forward") == 1
        assert names.count("backward") == 1
        assert names.count("optimizer") == 1
        assert "train.step" in names

    def test_hook_optimizer_advances_step(self):
        assert probing.step.micro_step == 0
        hook_enter(FORWARD)
        hook_exit(FORWARD)
        hook_enter(OPTIMIZER)
        hook_exit(OPTIMIZER)
        assert probing.step.micro_step == 1

    def test_hook_forward_records_train_step_duration(self):
        hook_enter(FORWARD)
        hook_exit(FORWARD)
        hook_enter(OPTIMIZER)
        hook_exit(OPTIMIZER)

        rows = _trace_rows()
        train_step = next(
            (
                r
                for r in rows
                if r.get("record_type") == "span_start"
                and r.get("name") == "train.step"
            ),
            None,
        )
        assert train_step is not None
        assert int(train_step.get("time", 0)) >= 0

    def test_hook_without_forward_skips_train_step(self):
        hook_enter(OPTIMIZER)
        hook_exit(OPTIMIZER)
        names = _closed_span_names(_trace_rows())
        assert "train.step" not in names

    def test_manual_span_suppresses_hook_span_but_keeps_phase(self):
        with probing.span("forward", phase=FORWARD):
            assert phase() == FORWARD
            hook_enter(FORWARD)
            assert phase() == FORWARD
            rows_mid = _trace_rows()
            forward_starts = [
                r
                for r in rows_mid
                if r.get("record_type") == "span_start" and r.get("name") == "forward"
            ]
            assert len(forward_starts) == 1
            hook_exit(FORWARD)
            assert phase() == FORWARD
        assert phase() == IDLE

    def test_hook_phase_spans_use_phase_hook_source(self):
        hook_enter(BACKWARD)
        rows = _trace_rows()
        backward = next(
            r
            for r in rows
            if r.get("record_type") == "span_start" and r.get("name") == "backward"
        )
        import json

        attrs = json.loads(backward.get("attributes") or "{}")
        assert attrs.get("source") == "phase_hook"
        hook_exit(BACKWARD)


# --- Span + hook collaboration ---


class TestSpanHookCollaboration:
    def test_manual_then_hook_optimizer_single_step_advance(self):
        with probing.span("forward", phase=FORWARD):
            pass
        with probing.span("backward", phase=BACKWARD):
            pass
        with probing.span("step", phase=OPTIMIZER):
            hook_enter(OPTIMIZER)
            assert phase() == OPTIMIZER
            hook_exit(OPTIMIZER)
        assert phase() == IDLE
        assert probing.step.micro_step == 1

    def test_hook_cycle_then_manual_spans_reset_correctly(self):
        hook_enter(FORWARD)
        hook_exit(FORWARD)
        assert phase() == IDLE

        with probing.span("backward", phase=BACKWARD):
            assert phase() == BACKWARD
        assert phase() == IDLE

    def test_phase_reads_span_stack_not_stale_after_hook_exit(self):
        with probing.span("forward", phase=FORWARD):
            hook_enter(FORWARD)
            assert phase() == FORWARD
            hook_exit(FORWARD)
            assert phase() == FORWARD
        assert phase() == IDLE


class TestGradientAccumulation:
    def test_grad_acc_one_train_step_per_optimizer(self):
        probing.step(micro_batches=4)
        for micro in range(4):
            hook_enter(FORWARD)
            hook_exit(FORWARD)
            hook_enter(BACKWARD)
            hook_exit(BACKWARD)
            if micro < 3:
                assert phase() == IDLE
            else:
                hook_enter(OPTIMIZER)
                hook_exit(OPTIMIZER)
                assert phase() == IDLE

        names = _closed_span_names(_trace_rows())
        assert names.count("train.step") == 1
        assert probing.step.micro_step == 1
        assert probing.step.local_step == 0

        hook_enter(FORWARD)
        hook_exit(FORWARD)
        hook_enter(BACKWARD)
        hook_exit(BACKWARD)
        hook_enter(OPTIMIZER)
        hook_exit(OPTIMIZER)

        names = _closed_span_names(_trace_rows())
        assert names.count("train.step") == 2
        assert probing.step.micro_step == 2

    def test_train_step_attrs_include_accum_index(self):
        import json

        probing.step(micro_batches=2)
        hook_enter(FORWARD)
        hook_exit(FORWARD)
        hook_enter(BACKWARD)
        hook_exit(BACKWARD)
        hook_enter(OPTIMIZER)
        hook_exit(OPTIMIZER)

        row = next(
            r
            for r in _trace_rows()
            if r.get("name") == "train.step" and r.get("record_type") == "span_start"
        )
        attrs = json.loads(row["attributes"])
        assert attrs["micro_batches"] == 2
        assert "accum_index" in attrs
        assert "logical_step_pending" in attrs


class TestSpanApi:
    def test_span_phase_only_defaults_name(self):
        with probing.span(phase=FORWARD) as s:
            assert s.name == FORWARD
            assert s.phase == FORWARD
            assert phase() == FORWARD


class TestTorchProbeOwnership:
    def test_torch_probe_skips_training_phase_when_hooks_attached(self):
        import torch
        import torch.nn as nn

        from probing.profiling.torch_probe import TorchProbe, TorchProbeConfig
        from probing.tracing.hooks import attach_training_phases, detach_training_phases

        class M(nn.Module):
            def __init__(self):
                super().__init__()
                self.w = nn.Parameter(torch.zeros(1))

            def forward(self, x):
                return self.w * x

        model = M()
        opt = torch.optim.SGD(model.parameters(), lr=0.1)
        attach_training_phases(model, opt)
        try:
            tracer = TorchProbe(config=TorchProbeConfig(enabled=True))
            tracer.log_module_stage("pre forward", model, force=True)
            tracer.log_module_stage("post forward", model, force=True)
            starts = [
                r
                for r in _trace_rows()
                if r.get("record_type") == "span_start" and r.get("phase") == FORWARD
            ]
            assert starts == []
        finally:
            detach_training_phases(model, opt)

    def test_torch_probe_optimizer_span_skipped_when_hooks_attached(self):
        import torch
        import torch.nn as nn

        from probing.profiling.torch_probe import TorchProbe, TorchProbeConfig
        from probing.tracing.hooks import attach_training_phases, detach_training_phases

        class M(nn.Module):
            def __init__(self):
                super().__init__()
                self.w = nn.Parameter(torch.zeros(1))

            def forward(self, x):
                return self.w * x

        model = M()
        opt = torch.optim.SGD(model.parameters(), lr=0.1)
        attach_training_phases(model, opt)
        try:
            tracer = TorchProbe(config=TorchProbeConfig(enabled=True))
            tracer._begin_train_step_span(optimizer=opt)
            assert tracer._train_step_cm is None
        finally:
            detach_training_phases(model, opt)

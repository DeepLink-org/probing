"""Phase tracker: model + optimizer hooks drive training spans."""

from __future__ import annotations

import dataclasses

import pytest
import torch
import torch.nn as nn
import torch.nn.functional as F

import probing


@pytest.fixture(autouse=True)
def _reset_trace_and_phases():
    from probing.profiling import phase_tracker
    from probing.tracing import TraceEvent, bind_table, reset_backends
    from probing.tracing.phases import reset_phase

    for (_mid, _oid), tracker in list(phase_tracker._REGISTRY.items()):
        phase_tracker.detach_training_phases(tracker.model, tracker.optimizer)
    phase_tracker._REGISTRY.clear()
    reset_phase()

    try:
        TraceEvent.drop()
    except Exception:
        pass
    TraceEvent.init_table()
    reset_backends(clear_registered=True)
    bind_table(TraceEvent)
    yield
    reset_backends(clear_registered=True)


def _trace_rows(n: int = 100) -> list[dict]:
    from probing.tracing import TraceEvent

    fields = [f.name for f in dataclasses.fields(TraceEvent)]
    return [dict(zip(fields, data)) for _ts, data in TraceEvent.take(n)]


def _closed_span_names(rows: list[dict]) -> list[str]:
    starts = {
        r["span_id"]: r["name"] for r in rows if r.get("record_type") == "span_start"
    }
    ends = {r["span_id"] for r in rows if r.get("record_type") == "span_end"}
    return [starts[sid] for sid in ends if sid in starts]


class TinyNet(nn.Module):
    def __init__(self) -> None:
        super().__init__()
        self.fc = nn.Linear(4, 2)

    def forward(self, x):
        return self.fc(x)


def test_phase_hooks_emit_training_spans():
    model = TinyNet()
    opt = torch.optim.SGD(model.parameters(), lr=0.01)
    probing.attach_training_phases(model, opt)

    x = torch.randn(2, 4)
    y = torch.tensor([0, 1])
    out = model(x)
    loss = F.cross_entropy(out, y)
    loss.backward()
    opt.step()

    names = _closed_span_names(_trace_rows())
    assert "forward" in names
    assert "backward" in names
    assert "optimizer" in names
    assert "train.step" in names


def test_phase_advances_on_optimizer_step():
    probing.step(0)
    model = TinyNet()
    opt = torch.optim.SGD(model.parameters(), lr=0.01)
    probing.attach_training_phases(model, opt)

    x = torch.randn(2, 4)
    y = torch.tensor([0, 1])
    out = model(x)
    loss = F.cross_entropy(out, y)
    loss.backward()
    opt.step()

    assert probing.step.micro_step == 1


def test_phase_idle_outside_training():
    assert probing.phase() == "idle"


def test_span_phase_drives_state_without_hooks():
    from probing.tracing.phases import FORWARD

    with probing.span("forward", phase=FORWARD):
        assert probing.phase() == "forward"
    assert probing.phase() == "idle"


def test_manual_forward_span_suppresses_hook_duplicate():
    model = TinyNet()
    opt = torch.optim.SGD(model.parameters(), lr=0.01)
    probing.attach_training_phases(model, opt)

    with probing.span("batch"):
        x = torch.randn(2, 4)
        y = torch.tensor([0, 1])
        with probing.span("forward"):
            out = model(x)
        loss = F.cross_entropy(out, y)
        loss.backward()
        opt.step()

    rows = _trace_rows()
    forward_starts = [
        r
        for r in rows
        if r.get("record_type") == "span_start" and r.get("name") == "forward"
    ]
    assert len(forward_starts) == 1

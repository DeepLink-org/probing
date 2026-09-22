from __future__ import annotations

import dataclasses
import math
import pathlib
import re

import pytest

from probing.ext.rl_data.protocol import CANONICAL_METRICS
from probing.ext.rl_data.tables import RlBenchmark, RlMetric, RlRun, RlSample
from probing.ext.rl_data.writer import RlTelemetryWriter


def test_canonical_tables_have_stable_identity_columns():
    assert [field.name for field in dataclasses.fields(RlRun)][:3] == [
        "timestamp_ns",
        "run_id",
        "framework",
    ]
    assert {"step", "name", "value", "labels_json", "rank"} <= {
        field.name for field in dataclasses.fields(RlMetric)
    }
    assert {"rollout_id", "group_id", "reward", "staleness"} <= {
        field.name for field in dataclasses.fields(RlSample)
    }
    assert "reward.mean" in CANONICAL_METRICS
    assert "cost.usd_total" in CANONICAL_METRICS
    assert "hardware.restart_count" in CANONICAL_METRICS


def test_dashboard_labels_every_canonical_metric():
    """The dashboard's metric table must cover every name we define here.

    `CANONICAL_METRICS` is the contract adapters write against; the frontend table
    supplies each name's short label and the hover text. A name present here but
    missing there renders as an untitled "RL metric" with no explanation.
    """

    overview = pathlib.Path(__file__).parents[5] / "web/src/next/pages/rl_overview.rs"
    if not overview.is_file():  # wheel-only test runs have no web sources
        pytest.skip("web sources not present")
    block = overview.read_text().split("const OVERVIEW_METRICS")[1].split("];")[0]
    # Tolerate rustfmt splitting a long row across several lines.
    string = r'"((?:[^"\\]|\\.)*)"'
    rows = re.findall(rf"\(\s*{string}\s*,\s*{string}\s*,\s*{string}\s*,?\s*\)", block)
    labelled = {name: (label, description) for name, label, description in rows}

    assert not set(CANONICAL_METRICS) - set(labelled), (
        "canonical metrics with no dashboard label"
    )
    assert not set(labelled) - set(CANONICAL_METRICS), (
        "dashboard labels with no canonical metric"
    )
    for name, (label, description) in labelled.items():
        assert label.strip(), f"{name} has a blank label"
        assert description == CANONICAL_METRICS[name], (
            f"{name} description drifted from protocol.py"
        )


def test_append_metrics_keeps_only_finite_numeric_values(monkeypatch):
    captured = []
    monkeypatch.setattr(RlMetric, "append_many", lambda rows: captured.extend(rows))
    writer = RlTelemetryWriter("xtuner", "run-1", rank=3, source="test")

    written = writer.append_metrics(
        7,
        {"reward.mean": 0.5, "skip": "not-a-number", "nan": math.nan},
        timestamp_ns=123,
        wall_time_s=2.0,
        labels={"task": "math"},
    )

    assert written == 1
    assert len(captured) == 1
    row = captured[0]
    assert row.run_id == "run-1"
    assert row.framework == "xtuner"
    assert row.step == 7
    assert row.rank == 3
    assert row.name == "reward.mean"
    assert row.labels_json == '{"task": "math"}'


def test_write_run_preserves_bound_metadata(monkeypatch):
    captured = []
    monkeypatch.setattr(RlRun, "save", lambda row: captured.append(row) or True)
    writer = RlTelemetryWriter("demo", "run-1", rank=0)
    writer.bind_run(
        "run-1",
        job_id="job-9",
        config_hash="abc",
        metadata={"lr": 1e-6, "model": "demo"},
        start_time_ns=10,
    )
    writer.write_run(phase="training", global_step=3, samples_total=8, tokens_total=100)
    assert len(captured) == 2
    latest = captured[-1]
    assert latest.job_id == "job-9"
    assert latest.config_hash == "abc"
    assert latest.samples_total == 8
    assert latest.tokens_total == 100
    assert (
        '"model": "demo"' in latest.metadata_json
        or '"model":"demo"' in latest.metadata_json
    )
    writer.write_run(phase="rollout", global_step=4)
    assert captured[-1].samples_total == 8
    assert captured[-1].tokens_total == 100
    writer.bump_totals(samples=2, tokens=50)
    writer.write_run(phase="training", global_step=5)
    assert captured[-1].samples_total == 10
    assert captured[-1].tokens_total == 150


def test_writer_is_fail_open(monkeypatch):
    def fail(_rows):
        raise RuntimeError("storage unavailable")

    monkeypatch.setattr(RlMetric, "append_many", fail)
    writer = RlTelemetryWriter("xtuner", "run-1", rank=0)
    assert writer.append_metrics(1, {"reward.mean": 1.0}) == 0


def test_sample_normalizes_mapping_reward(monkeypatch):
    captured = []
    monkeypatch.setattr(RlSample, "append", lambda row: captured.append(row))
    writer = RlTelemetryWriter("xtuner", "run-1", rank=0)

    assert writer.append_sample(
        {
            "rollout_id": "r-1",
            "group_id": "g-1",
            "reward": {"score": 0.75, "pass": True},
            "seq_staleness": 2,
        },
        timestamp_ns=123,
    )
    assert captured[0].sample_id == "r-1"
    assert captured[0].reward == 0.75
    assert captured[0].reward_pass is True
    assert captured[0].staleness == 2


def test_sample_derives_the_verdict_from_a_numeric_reward(monkeypatch):
    """A bare number is how XTuner's RL trainer reports reward.

    With no explicit verdict alongside it, treating the sample as failed makes
    every pass-rate view read zero, so a positive score has to count as a pass.
    """

    captured = []
    monkeypatch.setattr(RlSample, "append", lambda row: captured.append(row))
    writer = RlTelemetryWriter("xtuner", "run-1", rank=0)

    assert writer.append_sample({"rollout_id": "r-1", "reward": 1.0})
    assert captured[-1].reward == 1.0
    assert captured[-1].reward_pass is True

    assert writer.append_sample({"rollout_id": "r-2", "reward": 0.0})
    assert captured[-1].reward_pass is False

    # An explicit verdict still wins over the score.
    assert writer.append_sample({"rollout_id": "r-3", "reward": 1.0, "pass": False})
    assert captured[-1].reward_pass is False

    # Nothing judged at all stays a non-pass rather than guessing.
    assert writer.append_sample({"rollout_id": "r-4"})
    assert captured[-1].reward_pass is False


def test_sample_category_falls_back_to_task_namespace(monkeypatch):
    captured = []
    monkeypatch.setattr(RlSample, "append", lambda row: captured.append(row))
    writer = RlTelemetryWriter("xtuner", "run-1", rank=0)

    assert writer.append_sample({"task": "openai/gsm8k"})
    assert captured[-1].task == "openai/gsm8k"
    assert captured[-1].category == "openai"

    # An explicit category always wins over the namespace guess.
    assert writer.append_sample({"task": "openai/gsm8k", "category": "reasoning"})
    assert captured[-1].category == "reasoning"

    # A flat task name has no namespace to fall back on.
    assert writer.append_sample({"task": "gsm8k"})
    assert captured[-1].category == ""


def test_benchmark_records_harness_and_aggregation(monkeypatch):
    captured = []
    monkeypatch.setattr(RlBenchmark, "save", lambda row: captured.append(row))
    writer = RlTelemetryWriter("xtuner", "run-1", rank=0)

    assert writer.append_benchmark(
        "DeepSWE",
        30,
        72.57,
        sample_count=500,
        version="v1.1",
        harness="mini-swe-agent",
        aggregation="avg@3",
    )
    assert captured[-1].harness == "mini-swe-agent"
    assert captured[-1].aggregation == "avg@3"
    assert captured[-1].version == "v1.1"

"""Framework-neutral relational tables for RL observability."""

from __future__ import annotations

from dataclasses import dataclass, field

from probing.core import table


def _doc(text: str):
    return field(metadata={"doc": text})


@table("rl.run")
@dataclass
class RlRun:
    """Latest reported state of an RL training run."""

    timestamp_ns: int = _doc("Wall-clock time when this snapshot was recorded")
    run_id: str = _doc("Stable identifier for one training run")
    framework: str = _doc("Source framework, for example xtuner or slime")
    job_id: str = _doc("Scheduler or framework job identifier when available")
    phase: str = _doc("Current run phase such as rollout, training, or evaluation")
    global_step: int = _doc("Latest trainer global step")
    start_time_ns: int = _doc("Run start wall-clock time, or zero when unknown")
    end_time_ns: int = _doc("Run end wall-clock time, or zero while active")
    config_hash: str = _doc("Stable hash of the effective training configuration")
    metadata_json: str = _doc("Optional run metadata encoded as a JSON object")
    samples_total: int = _doc("Cumulative trained samples")
    tokens_total: int = _doc("Cumulative trained tokens")
    rank: int = _doc("Reporting distributed rank")


@table("rl.metric")
@dataclass
class RlMetric:
    """A framework-neutral scalar metric emitted by an RL trainer."""

    timestamp_ns: int = _doc("Wall-clock time when the scalar was recorded")
    run_id: str = _doc("Training run identifier")
    framework: str = _doc("Source training framework")
    step: int = _doc("Trainer global step")
    wall_time_s: float = _doc("Seconds elapsed since run start, or -1 when unknown")
    name: str = _doc("Canonical or source metric name")
    value: float = _doc("Numeric metric value")
    labels_json: str = _doc("Additional dimensions encoded as a JSON object")
    source: str = _doc("Producer of this row, for example exp_tracker")
    rank: int = _doc("Reporting distributed rank")


@table("rl.sampler")
@dataclass
class RlSampler:
    """A point-in-time snapshot of an RL sampling pipeline."""

    timestamp_ns: int = _doc("Wall-clock time when the snapshot was recorded")
    run_id: str = _doc("Training run identifier")
    framework: str = _doc("Source training framework")
    step: int = _doc("Trainer global step")
    target: int = _doc("Target number of prompts or samples")
    accepted: int = _doc("Accepted prompts or samples")
    judged: int = _doc("Samples with completed judging")
    trained: int = _doc("Samples admitted to training")
    filtered: int = _doc("Samples removed by a filter")
    failed: int = _doc("Samples lost to execution or infrastructure failures")
    expired: int = _doc("Samples expired because of staleness or window policy")
    in_flight: int = _doc("Samples still being processed")
    rank: int = _doc("Reporting distributed rank")


@table("rl.sample")
@dataclass
class RlSample:
    """The latest reported business outcome for one rollout sample."""

    timestamp_ns: int = _doc("Wall-clock time when this outcome was recorded")
    run_id: str = _doc("Training run identifier")
    framework: str = _doc("Source training framework")
    step: int = _doc("Trainer step associated with the sample")
    rollout_id: str = _doc("Rollout identifier")
    group_id: str = _doc("Prompt or rollout group identifier")
    sample_id: str = _doc("Framework-neutral sample identifier")
    task: str = _doc("Task or data-source label")
    category: str = _doc("Coarse grouping for the task, such as code or reasoning")
    status: str = _doc("Final or current sample status")
    reward: float = _doc("Scalar reward, or NaN when unavailable")
    reward_pass: bool = _doc("Whether the sample passed its judge")
    filter_reason: str = _doc("Reason the sample was filtered")
    drop_reason: str = _doc("Reason the sample was dropped before training")
    prompt_tokens: int = _doc("Prompt token count")
    response_tokens: int = _doc("Generated token count")
    staleness: int = _doc("Policy-version distance between sampling and training")
    rank: int = _doc("Reporting distributed rank")


@table("rl.benchmark")
@dataclass
class RlBenchmark:
    """A scalar result from a versioned evaluation benchmark."""

    timestamp_ns: int = _doc("Wall-clock time when the result was recorded")
    run_id: str = _doc("Training run identifier")
    framework: str = _doc("Source training framework")
    name: str = _doc("Benchmark name")
    step: int = _doc("Trainer step evaluated")
    score: float = _doc("Reported benchmark score")
    sample_count: int = _doc("Number of evaluated samples")
    version: str = _doc("Benchmark dataset or harness version")
    harness: str = _doc("Evaluation harness that produced the score")
    aggregation: str = _doc("How repeats were combined, such as avg@3 or pass@1")
    rank: int = _doc("Reporting distributed rank")


@table("rl.notice")
@dataclass
class RlNotice:
    """An operator-authored note explaining an intervention on a run."""

    timestamp_ns: int = _doc("Wall-clock time when the notice was filed")
    run_id: str = _doc("Training run identifier, or empty when run-agnostic")
    framework: str = _doc("Source training framework")
    step: int = _doc("Trainer step the notice refers to, or -1 when unknown")
    level: str = _doc("Severity such as info, warning, or error")
    kind: str = _doc("Notice category such as restart, dataset, or config")
    message: str = _doc("Operator-authored description of what happened")
    author: str = _doc("Who filed the notice")
    rank: int = _doc("Reporting distributed rank")


RL_TABLES = (RlRun, RlMetric, RlSampler, RlSample, RlBenchmark, RlNotice)


def init_tables() -> None:
    """Create every RL table eagerly."""

    for table_cls in RL_TABLES:
        table_cls.init_table()


def drop_tables() -> None:
    """Drop every RL table."""

    for table_cls in reversed(RL_TABLES):
        table_cls.drop()


__all__ = [
    "RL_TABLES",
    "RlBenchmark",
    "RlMetric",
    "RlNotice",
    "RlRun",
    "RlSample",
    "RlSampler",
    "drop_tables",
    "init_tables",
]

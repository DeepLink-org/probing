"""Fail-open writer for framework-neutral RL telemetry."""

from __future__ import annotations

import json
import logging
import math
import os
import time
from collections.abc import Mapping
from typing import Any, Optional

from .tables import RlBenchmark, RlMetric, RlNotice, RlRun, RlSample, RlSampler

logger = logging.getLogger(__name__)


def _int(value: Any, default: int = 0) -> int:
    try:
        return int(value)
    except (TypeError, ValueError, OverflowError):
        return default


def _float(value: Any, default: float = float("nan")) -> float:
    if isinstance(value, bool):
        return default
    try:
        result = float(value)
    except (TypeError, ValueError, OverflowError):
        return default
    return result if math.isfinite(result) else default


def _text(value: Any) -> str:
    return "" if value is None else str(value)


def _json(value: Any) -> str:
    try:
        return json.dumps(value or {}, ensure_ascii=False, sort_keys=True, default=str)
    except Exception:
        return "{}"


def _metadata_json(value: Any) -> str:
    if value is None:
        return "{}"
    if isinstance(value, str):
        text = value.strip()
        return text if text else "{}"
    return _json(value)


def _category_from_task(task: str) -> str:
    """Fall back to the task's namespace when an adapter reports no category.

    Semantic grouping (code, reasoning, agent) is the adapter's call; all this
    can do is expose the source hierarchy already present in names such as
    ``openai/gsm8k``.
    """

    namespace, separator, _ = task.partition("/")
    return namespace if separator and namespace else ""


def _rank() -> int:
    return _int(os.environ.get("RANK", os.environ.get("PROBING_RANK", -1)), -1)


def _now_ns(value: Optional[int] = None) -> int:
    return int(value) if value is not None else time.time_ns()


class RlTelemetryWriter:
    """Write canonical RL rows without propagating observability failures."""

    def __init__(
        self,
        framework: str,
        run_id: str = "",
        *,
        rank: Optional[int] = None,
        source: str = "adapter",
    ) -> None:
        self.framework = framework
        self.run_id = run_id
        self.rank = _rank() if rank is None else int(rank)
        self.source = source
        self.start_time_ns = 0
        self.job_id = ""
        self.config_hash = ""
        self.metadata_json = "{}"
        self.samples_total = 0
        self.tokens_total = 0
        self.phase = ""
        self.global_step = -1

    def _guard(self, operation: str, callback) -> bool:
        try:
            callback()
            return True
        except Exception:
            logger.debug("RL telemetry %s failed", operation, exc_info=True)
            return False

    def bump_totals(self, *, samples: int = 0, tokens: int = 0) -> None:
        """Accumulate trained sample and token counters for run cards."""

        self.samples_total = max(0, self.samples_total + max(0, _int(samples)))
        self.tokens_total = max(0, self.tokens_total + max(0, _int(tokens)))

    def bind_run(self, run_id: str, **metadata: Any) -> None:
        self.run_id = _text(run_id)
        self.start_time_ns = _int(metadata.get("start_time_ns"), self.start_time_ns)
        self.job_id = _text(metadata.get("job_id", self.job_id))
        self.config_hash = _text(metadata.get("config_hash", self.config_hash))
        if "samples_total" in metadata:
            self.samples_total = _int(metadata.get("samples_total"))
        if "tokens_total" in metadata:
            self.tokens_total = _int(metadata.get("tokens_total"))
        if "metadata" in metadata or "metadata_json" in metadata:
            self.metadata_json = _metadata_json(
                metadata.get("metadata", metadata.get("metadata_json"))
            )
        self.write_run(**metadata)

    def write_run(self, **metadata: Any) -> bool:
        timestamp_ns = _now_ns(metadata.get("timestamp_ns"))
        if "job_id" in metadata:
            self.job_id = _text(metadata.get("job_id"))
        if "config_hash" in metadata:
            self.config_hash = _text(metadata.get("config_hash"))
        if "metadata" in metadata or "metadata_json" in metadata:
            self.metadata_json = _metadata_json(
                metadata.get("metadata", metadata.get("metadata_json"))
            )
        if "samples_total" in metadata:
            self.samples_total = _int(metadata.get("samples_total"))
        if "tokens_total" in metadata:
            self.tokens_total = _int(metadata.get("tokens_total"))
        if "phase" in metadata:
            self.phase = _text(metadata.get("phase"))
        if "global_step" in metadata:
            self.global_step = _int(metadata.get("global_step"), self.global_step)
        if "start_time_ns" in metadata:
            self.start_time_ns = _int(metadata.get("start_time_ns"), self.start_time_ns)
        row = RlRun(
            timestamp_ns=timestamp_ns,
            run_id=_text(metadata.get("run_id", self.run_id)),
            framework=_text(metadata.get("framework", self.framework)),
            job_id=_text(metadata.get("job_id", self.job_id)),
            phase=_text(metadata.get("phase", self.phase)),
            global_step=_int(metadata.get("global_step"), self.global_step),
            start_time_ns=_int(metadata.get("start_time_ns"), self.start_time_ns),
            end_time_ns=_int(metadata.get("end_time_ns")),
            config_hash=_text(metadata.get("config_hash", self.config_hash)),
            metadata_json=_metadata_json(
                metadata.get(
                    "metadata", metadata.get("metadata_json", self.metadata_json)
                )
            ),
            samples_total=self.samples_total,
            tokens_total=self.tokens_total,
            rank=_int(metadata.get("rank"), self.rank),
        )
        return self._guard("write_run", row.save)

    def append_metrics(
        self,
        step: int,
        scalars: Mapping[str, Any],
        *,
        timestamp_ns: Optional[int] = None,
        wall_time_s: Optional[float] = None,
        labels: Optional[Mapping[str, Any]] = None,
        source: Optional[str] = None,
    ) -> int:
        recorded_at = _now_ns(timestamp_ns)
        if wall_time_s is None and self.start_time_ns > 0:
            wall_time_s = max(0.0, (recorded_at - self.start_time_ns) / 1e9)
        elapsed = _float(wall_time_s, -1.0)
        rows = []
        for name, raw_value in scalars.items():
            value = _float(raw_value)
            if not math.isfinite(value):
                continue
            rows.append(
                RlMetric(
                    timestamp_ns=recorded_at,
                    run_id=self.run_id,
                    framework=self.framework,
                    step=_int(step),
                    wall_time_s=elapsed,
                    name=_text(name),
                    value=value,
                    labels_json=_json(labels),
                    source=_text(source or self.source),
                    rank=self.rank,
                )
            )
        if not rows:
            return 0
        return (
            len(rows)
            if self._guard("append_metrics", lambda: RlMetric.append_many(rows))
            else 0
        )

    def append_sampler(
        self,
        step: int,
        counts: Mapping[str, Any],
        *,
        timestamp_ns: Optional[int] = None,
    ) -> bool:
        row = RlSampler(
            timestamp_ns=_now_ns(timestamp_ns),
            run_id=self.run_id,
            framework=self.framework,
            step=_int(step),
            target=_int(counts.get("target")),
            accepted=_int(counts.get("accepted")),
            judged=_int(counts.get("judged")),
            trained=_int(counts.get("trained")),
            filtered=_int(counts.get("filtered")),
            failed=_int(counts.get("failed")),
            expired=_int(counts.get("expired")),
            in_flight=_int(counts.get("in_flight")),
            rank=self.rank,
        )
        return self._guard("append_sampler", row.save)

    def append_sample(
        self,
        sample: Mapping[str, Any],
        *,
        timestamp_ns: Optional[int] = None,
    ) -> bool:
        reward_raw = sample.get("reward")
        reward_pass = sample.get("reward_pass", sample.get("pass"))
        if isinstance(reward_raw, Mapping):
            reward_pass = reward_raw.get("pass", reward_pass)
            reward_raw = reward_raw.get("score")
        if reward_pass is None:
            # A plain numeric reward carries no verdict of its own. Treat a
            # positive score as a pass, which is how RL trainers themselves read
            # it; without this every sample from such a trainer counts as failed
            # and the pass-rate views all read zero.
            reward_pass = _float(reward_raw) > 0.0 if reward_raw is not None else False
        task = _text(sample.get("task", sample.get("task_name")))
        row = RlSample(
            timestamp_ns=_now_ns(timestamp_ns),
            run_id=_text(sample.get("run_id", self.run_id)),
            framework=_text(sample.get("framework", self.framework)),
            step=_int(sample.get("step", sample.get("global_step", -1)), -1),
            rollout_id=_text(sample.get("rollout_id")),
            group_id=_text(sample.get("group_id")),
            sample_id=_text(
                sample.get(
                    "sample_id", sample.get("trajectory_id", sample.get("rollout_id"))
                )
            ),
            task=task,
            category=_text(sample.get("category")) or _category_from_task(task),
            status=_text(sample.get("status")),
            reward=_float(reward_raw),
            reward_pass=bool(reward_pass),
            filter_reason=_text(sample.get("filter_reason")),
            drop_reason=_text(sample.get("drop_reason")),
            prompt_tokens=_int(sample.get("prompt_tokens")),
            response_tokens=_int(sample.get("response_tokens")),
            staleness=_int(sample.get("staleness", sample.get("seq_staleness"))),
            rank=_int(sample.get("rank"), self.rank),
        )
        return self._guard("append_sample", row.save)

    def append_benchmark(
        self,
        name: str,
        step: int,
        score: float,
        *,
        sample_count: int = 0,
        version: str = "",
        harness: str = "",
        aggregation: str = "",
        timestamp_ns: Optional[int] = None,
    ) -> bool:
        row = RlBenchmark(
            timestamp_ns=_now_ns(timestamp_ns),
            run_id=self.run_id,
            framework=self.framework,
            name=_text(name),
            step=_int(step),
            score=_float(score),
            sample_count=_int(sample_count),
            version=_text(version),
            harness=_text(harness),
            aggregation=_text(aggregation),
            rank=self.rank,
        )
        return self._guard("append_benchmark", row.save)

    def append_notice(
        self,
        message: str,
        *,
        step: int = -1,
        level: str = "info",
        kind: str = "operator",
        author: str = "",
        timestamp_ns: Optional[int] = None,
    ) -> bool:
        row = RlNotice(
            timestamp_ns=_now_ns(timestamp_ns),
            run_id=self.run_id,
            framework=self.framework,
            step=_int(step, -1),
            level=_text(level),
            kind=_text(kind),
            message=_text(message),
            author=_text(author),
            rank=self.rank,
        )
        return self._guard("append_notice", row.save)


def notice(
    message: str,
    *,
    run_id: str = "",
    framework: str = "",
    step: int = -1,
    level: str = "info",
    kind: str = "operator",
    author: str = "",
) -> bool:
    """File an operator note that the RL dashboard shows alongside events.

    Use this to record *why* a run was restarted or reconfigured — the
    automatically synthesized events can only report that it happened.
    """

    writer = RlTelemetryWriter(framework, run_id, source="operator")
    return writer.append_notice(
        message, step=step, level=level, kind=kind, author=author
    )


__all__ = ["RlTelemetryWriter", "notice"]

"""XTuner adapter for framework-neutral Probing RL telemetry.

The adapter hooks XTuner's existing experiment writer instead of duplicating
trainer metric construction. All telemetry paths are fail-open.
"""

from __future__ import annotations

import functools
import logging
import math
import os
import re
import types
from collections import defaultdict
from collections.abc import Mapping
from typing import Any, Optional

from probing.ext.rl_data.writer import RlTelemetryWriter

framework = "xtuner"

_ORIGINAL_LOG_STEP = None
_ORIGINAL_SAVE_TRAJECTORIES = None
_ORIGINAL_RUN_INITIAL_EVALUATE = None
_ENABLED = False

# XTuner methods this adapter can hook. Versions differ in which of them exist.
_HOOK_POINTS = ("_log_step", "_save_trajectories", "_run_initial_evaluate")
_INSTALLED_HOOKS: list[str] = []
_INSTALL_ERROR = ""

_EXACT_CANONICAL_NAMES = {
    # Reward and advantage distribution over the rollout batch.
    "response/rewards/mean": "reward.mean",
    "response/rewards/min": "reward.min",
    "response/rewards/max": "reward.max",
    "response/raw_rewards/mean": "reward.raw_mean",
    "response/advantages/mean": "advantage.mean",
    "response/advantages/min": "advantage.min",
    "response/advantages/max": "advantage.max",
    "response/batch_size": "sampler.batch_size",
    # Context window usage.
    "response/prompt_len/mean": "context.prompt_tokens.mean",
    "response/prompt_len/min": "context.prompt_tokens.min",
    "response/prompt_len/max": "context.prompt_tokens.max",
    "response/response_len/mean": "context.response_tokens.mean",
    "response/response_len/min": "context.response_tokens.min",
    "response/response_len/max": "context.response_tokens.max",
    "response/response_len/std": "context.response_tokens.std",
    "response/tool_turns/mean": "agent.turns.mean",
    # Policy entropy on the trainer and on the inference engine.
    "entropy/train": "policy.entropy",
    "entropy/rollout": "policy.entropy_rollout",
    # Step wall-clock breakdown.
    "time/step": "time.step_s",
    "time/produce_batch": "time.rollout_s",
    "time/get_batch": "time.rollout_s",
    "time/training": "time.training_s",
    "time/onload": "time.onload_s",
    "time/final_offload": "time.offload_s",
    "time/prepare_data": "time.prepare_data_s",
    "time/save_ckpt": "time.save_ckpt_s",
    "time/switch_to_rollout": "time.switch_to_rollout_s",
    # Per-task rollout latency spread; the p99/p50 ratio exposes stragglers.
    "timing/task_n": "sampler.task_count",
    "timing/task_mean_s": "sampler.task_mean_s",
    "timing/task_p50_s": "sampler.task_p50_s",
    "timing/task_p99_s": "sampler.task_p99_s",
    "timing/task_p99_p50_ratio": "sampler.task_p99_p50_ratio",
    # Throughput, separated by stage.
    "throughput/e2e_effective_sgs": "throughput.e2e_samples_s",
    "throughput/e2e_effective_tgs": "throughput.e2e_tokens_s",
    "throughput/effective_sgs": "throughput.effective_samples_s",
    "throughput/effective_tgs": "throughput.effective_tokens_s",
    "throughput/training_tgs": "throughput.training_tokens_s",
    "throughput/rollout_sgs": "throughput.rollout_samples_s",
    "throughput/rollout_tgs": "throughput.rollout_tokens_s",
    # Divergence between the trainer and the inference engine on same tokens.
    "mismatch/mismatch_kl": "policy.train_infer_kl",
    "mismatch/mismatch_k3_kl": "policy.train_infer_k3_kl",
    "mismatch/mismatch_logprob_abs_diff": "policy.train_infer_logprob_abs_diff",
    "mismatch/mismatch_log_ppl_diff": "policy.train_infer_log_ppl_diff",
    "mismatch/mismatch_log_ppl_abs_diff": "policy.train_infer_log_ppl_abs_diff",
    "mismatch/mismatch_log_ppl_diff_max": "policy.train_infer_log_ppl_diff_max",
    "mismatch/mismatch_log_ppl_diff_min": "policy.train_infer_log_ppl_diff_min",
    "mismatch/mismatch_ppl_ratio": "policy.train_infer_ppl_ratio",
    "mismatch/mismatch_rollout_ppl": "policy.rollout_ppl",
    "mismatch/mismatch_rollout_log_ppl": "policy.rollout_log_ppl",
    "mismatch/mismatch_training_ppl": "policy.training_ppl",
    "mismatch/mismatch_training_log_ppl": "policy.training_log_ppl",
}

# Suffixes of `train_metrics/worker_0/...` scalars. Longest suffix wins, so
# `local_base_loss` is not shadowed by the bare `loss` entry.
_WORKER_SUFFIX_NAMES = {
    "grad_norm": "policy.grad_norm",
    "loss": "policy.pg_loss",
    "pg_loss": "policy.pg_loss",
    "policy_loss": "policy.pg_loss",
    "reduced_llm_loss": "policy.pg_loss",
    "local_base_loss": "policy.base_loss",
    "reduced_train_policy_clip_frac_high": "policy.clip_frac_high",
    "reduced_train_policy_clip_frac_low": "policy.clip_frac_low",
    "reduced_train_policy_kl1": "policy.kl1",
    "reduced_train_policy_kl3": "policy.kl3",
    "reduced_train_policy_ratio_abs_dev_mean": "policy.ratio_abs_dev_mean",
    "reduced_train_policy_ratio_max": "policy.ratio_max",
    "reduced_train_policy_ratio_min": "policy.ratio_min",
    "step_consumed_tokens": "train.tokens",
    "step_consumed_img_tokens": "train.img_tokens",
    "step_seqlen_tokens": "train.seqlen_tokens",
    "efficient_attn_ratio": "train.efficient_attn_ratio",
    "img_efficient_attn_ratio": "train.img_efficient_attn_ratio",
    "max_memory": "hardware.max_memory_gb",
    "reserved_memory": "hardware.reserved_memory_gb",
}

_WORKER_SUFFIXES_LONGEST_FIRST = sorted(_WORKER_SUFFIX_NAMES, key=len, reverse=True)


def _worker_canonical_name(lowered: str) -> Optional[str]:
    """Resolve a rank-0 train-worker scalar to its canonical name."""

    if "worker_0/" not in lowered:
        return None
    for suffix in _WORKER_SUFFIXES_LONGEST_FIRST:
        if lowered.endswith(suffix):
            return _WORKER_SUFFIX_NAMES[suffix]
    return None


def canonicalize_scalars(scalars: Mapping[str, Any]) -> dict[str, Any]:
    """Map stable XTuner scalar names to the Probing RL vocabulary."""

    canonical: dict[str, Any] = {}
    for source_name, value in scalars.items():
        name = _EXACT_CANONICAL_NAMES.get(source_name)
        if name is not None:
            canonical.setdefault(name, value)
            continue
        lowered = source_name.lower()
        worker_name = _worker_canonical_name(lowered)
        if worker_name is not None:
            # `step_avg_` is the whole-step reduction; prefer it over the
            # per-microbatch value that shares the same canonical name.
            if "step_avg_" in lowered:
                canonical[worker_name] = value
            else:
                canonical.setdefault(worker_name, value)
            continue
        if "mismatch" in lowered and lowered.endswith(("kl", "kl_mean")):
            canonical.setdefault("policy.train_infer_kl", value)

    prompt = canonical.get("context.prompt_tokens.mean")
    response = canonical.get("context.response_tokens.mean")
    if prompt is not None and response is not None:
        try:
            canonical["context.total_tokens.mean"] = float(prompt) + float(response)
        except (TypeError, ValueError):
            pass
    return canonical


def _ops_metrics_from_env() -> dict[str, float]:
    """Optional cost / restart gauges injected via environment variables."""

    metrics: dict[str, float] = {}
    for env_name, metric_name in (
        ("PROBING_RL_COST_USD", "cost.usd_total"),
        ("PROBING_RL_COST_USD_PER_HOUR", "cost.usd_per_hour"),
        ("PROBING_RL_RESTARTS", "hardware.restart_count"),
        ("PROBING_RL_SANDBOX_TOTAL", "environment.sandbox_total"),
    ):
        raw = os.environ.get(env_name, "").strip()
        if not raw:
            continue
        try:
            metrics[metric_name] = float(raw)
        except ValueError:
            continue
    about_restarts = metrics.get("hardware.restart_count")
    if about_restarts is not None:
        metrics["about.restarts"] = about_restarts
    return metrics


class XtunerAdapter:
    """Translate XTuner experiment-writer callbacks into canonical RL rows."""

    framework = framework

    def __init__(self, writer: RlTelemetryWriter) -> None:
        self.writer = writer
        self.total_steps = 0
        self.group_size = 0.0
        self._last_sample_step = -1
        self._sandbox_total = 0
        self._env_leak_count = 0

    def bind_run(self, run_id: str, **metadata: Any) -> None:
        self.writer.bind_run(run_id, framework=self.framework, **metadata)

    def on_scalars(self, step: int, scalars: Mapping[str, Any], **context: Any) -> None:
        timestamp_ns = context.get("timestamp_ns")
        wall_time_s = context.get("wall_time_s")
        self.writer.append_metrics(
            step,
            scalars,
            timestamp_ns=timestamp_ns,
            wall_time_s=wall_time_s,
            source="xtuner.exp_tracker",
        )
        canonical = canonicalize_scalars(scalars)
        canonical.update(_ops_metrics_from_env())
        canonical.update(self._environment_from_scalars(scalars))
        canonical.update(self._progress_from_step(step, canonical))
        if self.group_size > 0:
            canonical.setdefault("sampler.group_size", self.group_size)
        self.writer.append_metrics(
            step,
            canonical,
            timestamp_ns=timestamp_ns,
            wall_time_s=wall_time_s,
            labels={"adapter": "xtuner"},
            source="xtuner.adapter",
        )
        self._emit_eval_benchmarks(step, scalars, timestamp_ns=timestamp_ns)
        # Prefer sample-derived totals; fall back to trainer token gauges only
        # when trajectories were not saved for this step.
        step_tokens = _number(canonical.get("train.tokens"))
        if step_tokens > 0 and self._last_sample_step != step:
            self.writer.bump_totals(tokens=step_tokens)
        phase = (
            "evaluation"
            if any(str(name).startswith("eval/") for name in scalars)
            else "training"
        )
        self.writer.write_run(global_step=step, phase=phase)
        self._write_sampler(step, scalars)

    def _progress_from_step(
        self, step: int, canonical: Mapping[str, Any]
    ) -> dict[str, float]:
        """Remaining-work gauges so a dashboard can show completion and ETA."""

        if self.total_steps <= 0 or step < 0:
            return {}
        completed = min(step, self.total_steps)
        metrics = {
            "progress.total_steps": float(self.total_steps),
            "progress.completed_steps": float(completed),
            "progress.completed_ratio": completed / float(self.total_steps),
        }
        remaining = self.total_steps - completed
        step_seconds = _seconds(canonical.get("time.step_s"))
        if remaining > 0 and step_seconds > 0.0:
            metrics["progress.eta_s"] = step_seconds * remaining
        return metrics

    def _environment_from_scalars(self, scalars: Mapping[str, Any]) -> dict[str, float]:
        """Derive sandbox / queue gauges from async leftover counters."""

        init_n = _number(scalars.get("async/init_samples"))
        completed = _number(scalars.get("async/completed_samples"))
        failed = _number(scalars.get("async/failed_samples"))
        aborted = _number(scalars.get("async/aborted_samples"))
        active = init_n + completed
        metrics: dict[str, float] = {}
        if active > 0:
            metrics["environment.active"] = float(active)
        attempts = completed + failed + aborted
        if attempts > 0:
            metrics["environment.setup_error_ratio"] = float(failed + aborted) / float(
                attempts
            )
        queue_s = scalars.get("timing/pause_s")
        if queue_s is not None:
            try:
                metrics["environment.queue_s"] = float(queue_s)
            except (TypeError, ValueError):
                pass
        # Prefer live totals; env override still wins via later update order in
        # callers that merge `_ops_metrics_from_env` after this helper.
        if self._sandbox_total > 0:
            metrics.setdefault("environment.sandbox_total", float(self._sandbox_total))
        if self._env_leak_count > 0:
            metrics["environment.leak_count"] = float(self._env_leak_count)
        return metrics

    def _emit_eval_benchmarks(
        self,
        step: int,
        scalars: Mapping[str, Any],
        *,
        timestamp_ns: Any = None,
    ) -> None:
        for name, raw in scalars.items():
            text = str(name)
            if not text.startswith("eval/"):
                continue
            try:
                score = float(raw)
            except (TypeError, ValueError):
                continue
            if not math.isfinite(score):
                continue
            self.on_benchmark(
                text[len("eval/") :],
                step,
                score,
                timestamp_ns=timestamp_ns,
                version="xtuner-eval",
            )

    def _write_sampler(self, step: int, scalars: Mapping[str, Any]) -> None:
        # Sample-derived funnel snapshots are preferred for the same step.
        if self._last_sample_step == step:
            return
        keys = {
            "trained": "response/training_samples",
            "filtered": "async/filtered_samples",
            "failed": "async/failed_samples",
            "expired": "async/expired_samples",
            "target": "async/target_samples",
        }
        counts = {
            target: scalars[source]
            for target, source in keys.items()
            if source in scalars
        }
        completed = scalars.get("async/completed_samples")
        pending = sum(
            _number(scalars.get(name))
            for name in (
                "async/init_samples",
                "async/completed_samples",
                "async/aborted_samples",
                "async/expired_samples",
            )
        )
        if completed is not None:
            counts["accepted"] = completed
            counts.setdefault("judged", completed)
        if "target" not in counts:
            target = (
                _number(scalars.get("response/training_samples"))
                + _number(scalars.get("async/filtered_samples"))
                + _number(scalars.get("async/failed_samples"))
            )
            if target:
                counts["target"] = target
        meaningful = (
            any(
                _number(counts.get(name)) > 0
                for name in (
                    "target",
                    "accepted",
                    "judged",
                    "trained",
                    "filtered",
                    "failed",
                    "expired",
                )
            )
            or pending > 0
        )
        if not meaningful:
            return
        counts["in_flight"] = pending
        self.on_sampler_snapshot(step, counts)

    def on_sampler_snapshot(
        self, step: int, counts: Mapping[str, Any], **context: Any
    ) -> None:
        self.writer.append_sampler(
            step, counts, timestamp_ns=context.get("timestamp_ns")
        )

    def on_sample(self, sample: Mapping[str, Any], **context: Any) -> None:
        self.writer.append_sample(sample, timestamp_ns=context.get("timestamp_ns"))

    def on_samples(self, step: int, samples: list[Any]) -> None:
        """Write sample outcomes and metrics derived from rollout groups.

        Accepts either a flat list of samples or the trainer's batch of prompt
        groups. Grouping matters: pass rate is per prompt, so flattening the
        batch before it gets here loses the only grouping some trainers provide.
        """

        flattened = _flatten_groups(samples)
        groups: dict[str, list[bool]] = defaultdict(list)
        staleness = []
        token_count = 0
        trained = 0
        filtered = 0
        failed = 0
        for state, fallback_group in flattened:
            sample = _sample_mapping(state, step, fallback_group)
            self.on_sample(sample)
            status = str(sample.get("status") or "").lower()
            if "filter" in status:
                filtered += 1
            elif "fail" in status or "error" in status:
                failed += 1
            else:
                trained += 1
            group_id = str(sample.get("group_id") or sample.get("rollout_id") or "")
            reward = sample.get("reward")
            score = reward.get("score") if isinstance(reward, Mapping) else reward
            if group_id and score is not None:
                try:
                    groups[group_id].append(float(score) > 0)
                except (TypeError, ValueError):
                    pass
            stale = _number(sample.get("staleness"))
            staleness.append(stale)
            token_count += _number(sample.get("prompt_tokens")) + _number(
                sample.get("response_tokens")
            )

        derived: dict[str, float] = {}
        measurable = [attempts for attempts in groups.values() if attempts]
        if measurable:
            rates = [sum(attempts) / len(attempts) for attempts in measurable]
            derived["sampler.avg_pass"] = sum(rates) / len(rates)
            derived["sampler.pass_zero_ratio"] = sum(rate == 0 for rate in rates) / len(
                rates
            )
            derived["sampler.pass_one_ratio"] = sum(rate == 1 for rate in rates) / len(
                rates
            )
            derived["sampler.measurable_prompts"] = float(len(measurable))
        if staleness:
            derived["sampler.avg_staleness"] = sum(staleness) / len(staleness)
        if token_count:
            derived["train.tokens"] = float(token_count)
        if flattened:
            self._sandbox_total += len(flattened)
            derived["environment.active"] = float(len(flattened))
            derived["environment.sandbox_total"] = float(self._sandbox_total)
            total = len(flattened)
            derived["environment.setup_error_ratio"] = float(failed) / float(total)
            self._env_leak_count += failed
            derived["environment.leak_count"] = float(self._env_leak_count)
        ops = _ops_metrics_from_env()
        # Keep live sandbox totals unless the operator forces an override.
        if "environment.sandbox_total" in ops and self._sandbox_total > 0:
            ops.pop("environment.sandbox_total", None)
        derived.update(ops)
        self.writer.append_metrics(
            step,
            derived,
            labels={"adapter": "xtuner", "derived_from": "rollout_samples"},
            source="xtuner.adapter",
        )
        if "sampler.avg_pass" in derived:
            self.on_benchmark(
                "online/avg_pass",
                step,
                float(derived["sampler.avg_pass"]),
                sample_count=len(measurable) if measurable else len(flattened),
                version="xtuner-online",
            )
        if flattened:
            self._last_sample_step = step
            self.writer.bump_totals(samples=len(flattened), tokens=token_count)
            self.writer.write_run(global_step=step, phase="training")
            self.on_sampler_snapshot(
                step,
                {
                    "target": max(len(groups), len(flattened)),
                    "accepted": len(flattened),
                    "judged": len(flattened),
                    "trained": trained,
                    "filtered": filtered,
                    "failed": failed,
                    "expired": 0,
                    "in_flight": 0,
                },
            )

    def on_benchmark(self, name: str, step: int, score: float, **context: Any) -> None:
        self.writer.append_benchmark(
            name,
            step,
            score,
            sample_count=_number(context.get("sample_count")),
            version=str(context.get("version") or ""),
            timestamp_ns=context.get("timestamp_ns"),
        )


def _number(value: Any) -> int:
    try:
        return int(value)
    except (TypeError, ValueError, OverflowError):
        return 0


def _seconds(value: Any) -> float:
    try:
        result = float(value)
    except (TypeError, ValueError, OverflowError):
        return 0.0
    return result if math.isfinite(result) else 0.0


def _value(value: Any) -> Any:
    return getattr(value, "value", value)


def _flatten_groups(samples: Any) -> list[tuple[Any, str]]:
    """Pair each rollout sample with the id of the prompt group it came from.

    A trainer hands over its batch either already flattened or nested one level
    per prompt. The nesting is the only grouping some versions provide, so it is
    turned into an explicit id rather than discarded.
    """

    if not samples:
        return []
    flattened: list[tuple[Any, str]] = []
    for index, entry in enumerate(samples):
        nested = (
            isinstance(entry, (list, tuple))
            # A mapping is one sample, even though it is iterable.
            and not isinstance(entry, Mapping)
        )
        if nested:
            for state in entry:
                flattened.append((state, f"group-{index}"))
        else:
            flattened.append((entry, ""))
    return flattened


def _field(state: Any, name: str, default: Any = None) -> Any:
    """Read one field of a rollout sample.

    XTuner hands trajectories over as plain dicts in some versions and as
    objects in others. Reading only attributes silently yields defaults for the
    dict form, which writes a table full of blank samples.
    """

    if isinstance(state, Mapping):
        return state.get(name, default)
    return getattr(state, name, default)


def _task_label(state: Any, extra: Mapping[str, Any]) -> str:
    reward_model = _field(state, "reward_model")
    reward_model = reward_model if isinstance(reward_model, Mapping) else {}
    for candidate in (
        _field(state, "task_name"),
        extra.get("task_name"),
        _field(state, "data_source"),
        extra.get("data_source"),
        # Trajectories written by XTuner's RL trainer name their source here.
        reward_model.get("data_source"),
        reward_model.get("style"),
    ):
        if candidate is None or candidate == "":
            continue
        if isinstance(candidate, Mapping):
            # XTuner sometimes stores weighted source maps like {"openai/gsm8k": 1.0}.
            keys = [
                str(key) for key, weight in candidate.items() if _number(weight) > 0
            ]
            if keys:
                return "+".join(sorted(keys))
            return "+".join(sorted(str(key) for key in candidate.keys()))
        return str(candidate)
    return ""


def _category_label(extra: Mapping[str, Any]) -> str:
    """Read a coarse task grouping from whichever key the dataset config used."""

    for key in ("category", "data_category", "domain", "ability"):
        value = extra.get(key)
        if value not in (None, ""):
            return str(value)
    return ""


def _sample_mapping(state: Any, step: int, group_id: str = "") -> dict[str, Any]:
    extra = _field(state, "extra_fields")
    extra = extra if isinstance(extra, Mapping) else {}
    prompt_ids = _field(state, "prompt_ids")
    response_ids = _field(state, "response_ids")
    return {
        "step": step,
        "rollout_id": _field(state, "rollout_id", ""),
        # Fall back to the caller's grouping: trajectories carry their prompt
        # grouping in the nesting of the batch, not on each sample, and the
        # pass-rate views are per prompt.
        "group_id": _field(state, "group_id", "") or group_id,
        "sample_id": _field(state, "uid") or _field(state, "rollout_id", ""),
        "task_name": _task_label(state, extra),
        "category": _category_label(extra),
        "status": _value(_field(state, "status", "")),
        "reward": _field(state, "reward"),
        "filter_reason": extra.get("filter_reason", extra.get("filter.reason", "")),
        "drop_reason": extra.get("drop_reason", extra.get("drop.reason", "")),
        "prompt_tokens": _token_count(prompt_ids, _field(state, "messages")),
        "response_tokens": _token_count(response_ids, _field(state, "response_str")),
        "staleness": _field(state, "seq_staleness", 0),
    }


def _token_count(ids: Any, text: Any) -> int:
    """Token count when the trainer tokenised, else an approximation from text.

    Trajectories saved to disk keep the decoded strings rather than the ids, so
    without a fallback every context-length metric reads zero.
    """

    if ids is not None:
        try:
            return len(ids)
        except TypeError:
            return 0
    if isinstance(text, str):
        return _approx_tokens(text)
    if isinstance(text, (list, tuple)):
        return sum(
            _approx_tokens(str(part.get("content", "")))
            if isinstance(part, Mapping)
            else 0
            for part in text
        )
    return 0


def _approx_tokens(text: str) -> int:
    """Roughly four characters per token, but never zero for real text.

    A short answer such as "6" is still one token, and rounding it away makes a
    context-length metric read as if nothing was generated.
    """

    return max(1, len(text) // 4) if text else 0


def _step_from_save_path(save_path: Any) -> int:
    match = re.search(r"(\d+)(?!.*\d)", str(save_path))
    return int(match.group(1)) if match else -1


def _run_id(trainer: Any) -> str:
    explicit = os.environ.get("PROBING_RL_RUN_ID", "").strip()
    if explicit:
        return explicit
    try:
        exp_dir = trainer.exp_dir
        if exp_dir:
            return str(exp_dir)
    except Exception:
        pass
    return os.environ.get("RAY_JOB_ID", "").strip() or f"xtuner-{os.getpid()}"


def _adapter_for_trainer(trainer: Any) -> XtunerAdapter:
    adapter = getattr(trainer, "_probing_rl_adapter", None)
    if adapter is None:
        writer = RlTelemetryWriter(framework, _run_id(trainer), source="xtuner")
        adapter = XtunerAdapter(writer)
        adapter.bind_run(
            writer.run_id,
            job_id=os.environ.get("RAY_JOB_ID", ""),
            phase="initializing",
        )
        trainer._probing_rl_adapter = adapter
    # Resolved after trainer init, and may change when total_epochs is used.
    adapter.total_steps = _number(getattr(trainer, "_total_train_steps", 0))
    adapter.group_size = _group_size(trainer)
    return adapter


def _group_size(trainer: Any) -> float:
    """Responses sampled per prompt, under whichever name this XTuner uses."""

    names = (
        "prompt_repeat_k",
        "_prompt_repeat_k",
        "group_size",
        "_group_size",
        "n_samples_per_prompt",
    )
    holders = (
        trainer,
        getattr(trainer, "_config", None),
        getattr(trainer, "cfg", None),
        getattr(trainer, "_rollout_config", None),
    )
    for holder in holders:
        if holder is None:
            continue
        for name in names:
            value = _number(getattr(holder, name, 0))
            if value > 0:
                return value
    return 0.0


def _ensure_tracker_hook(trainer: Any) -> None:
    tracker = getattr(trainer, "_exp_tracker", None)
    if tracker is None or getattr(tracker, "_probing_rl_hooked", False):
        return
    original = getattr(tracker, "add_scalars", None)
    if not callable(original):
        return

    adapter = _adapter_for_trainer(trainer)

    @functools.wraps(original)
    def add_scalars_wrapper(_self, *args, **kwargs):
        result = original(*args, **kwargs)
        if _ENABLED:
            try:
                scalars = kwargs.get("tag_scalar_dict")
                step = kwargs.get("global_step")
                if scalars is None and args:
                    scalars = args[0]
                if step is None and len(args) > 1:
                    step = args[1]
                if isinstance(scalars, Mapping):
                    adapter.on_scalars(_number(step), scalars)
            except Exception:
                pass
        return result

    tracker.add_scalars = types.MethodType(add_scalars_wrapper, tracker)
    tracker._probing_rl_hooked = True
    tracker._probing_rl_adapter = adapter


def _record_install_failure(reason: str) -> None:
    """Remember and log why no hook could be installed.

    Every telemetry path here is fail-open, which means a version mismatch is
    indistinguishable from an idle trainer unless it is said out loud.
    """

    global _INSTALL_ERROR
    _INSTALL_ERROR = reason
    _INSTALLED_HOOKS.clear()
    logging.getLogger(__name__).warning(
        "probing RL telemetry for XTuner is inactive: %s", reason
    )


def status() -> dict[str, Any]:
    """What the adapter managed to hook, for diagnosing a silent run."""

    return {
        "enabled": _ENABLED,
        "installed_hooks": list(_INSTALLED_HOOKS),
        "error": _INSTALL_ERROR,
    }


def _resolve_rl_trainer_cls() -> Any | None:
    """Return the XTuner RL trainer class across package renames.

    Any one hook point is enough. Requiring a particular one meant a release that
    dropped or renamed it disabled every other hook too, and the adapter then
    reported nothing at all without saying why.
    """

    try:
        from xtuner.v1.train import rl_trainer as module
    except Exception:
        return None
    for name in ("BaseRLTrainer", "RLTrainer"):
        cls = getattr(module, name, None)
        if cls is not None and any(hasattr(cls, hook) for hook in _HOOK_POINTS):
            return cls
    return None


def install() -> bool:
    """Install the XTuner hooks, keeping whichever ones this version supports.

    Returns whether anything at all could be hooked. A version that supports only
    some hook points still gets those; only a complete mismatch counts as failure,
    and that is recorded in `installed_hooks` so it can be diagnosed rather than
    looking like a run that simply produced no telemetry.
    """

    global _ENABLED, _ORIGINAL_LOG_STEP, _ORIGINAL_SAVE_TRAJECTORIES
    global _ORIGINAL_RUN_INITIAL_EVALUATE
    if _ENABLED:
        return True
    try:
        trainer_cls = _resolve_rl_trainer_cls()
        if trainer_cls is None:
            _record_install_failure(
                "no XTuner RL trainer class with a known hook point; "
                "expected xtuner.v1.train.rl_trainer.RLTrainer with one of "
                f"{', '.join(_HOOK_POINTS)}"
            )
            return False
        _INSTALLED_HOOKS.clear()

        original = getattr(trainer_cls, "_log_step", None)
        if callable(original):
            if getattr(original, "_probing_rl_hook", False):
                _ENABLED = True
                return True

            @functools.wraps(original)
            def log_step_wrapper(self, *args, **kwargs):
                try:
                    _ensure_tracker_hook(self)
                except Exception:
                    pass
                return original(self, *args, **kwargs)

            log_step_wrapper._probing_rl_hook = True
            _ORIGINAL_LOG_STEP = original
            trainer_cls._log_step = log_step_wrapper
            _INSTALLED_HOOKS.append("_log_step")

        save_trajectories = getattr(trainer_cls, "_save_trajectories", None)
        if callable(save_trajectories) and not getattr(
            save_trajectories, "_probing_rl_hook", False
        ):

            @functools.wraps(save_trajectories)
            def save_trajectories_wrapper(
                self, data_groups, save_path, *args, **kwargs
            ):
                result = save_trajectories(
                    self, data_groups, save_path, *args, **kwargs
                )
                if _ENABLED:
                    try:
                        adapter = _adapter_for_trainer(self)
                        # Pass the groups as they are. Flattening here discards the
                        # prompt grouping, which is what pass rate is measured over
                        # and which the samples do not carry individually.
                        adapter.on_samples(_step_from_save_path(save_path), data_groups)
                    except Exception:
                        pass
                return result

            save_trajectories_wrapper._probing_rl_hook = True
            _ORIGINAL_SAVE_TRAJECTORIES = save_trajectories
            trainer_cls._save_trajectories = save_trajectories_wrapper
            _INSTALLED_HOOKS.append("_save_trajectories")

        run_initial = getattr(trainer_cls, "_run_initial_evaluate", None)
        if callable(run_initial) and not getattr(
            run_initial, "_probing_rl_hook", False
        ):

            @functools.wraps(run_initial)
            async def run_initial_wrapper(self, *args, **kwargs):
                # Initial eval logs eval/* before the first _log_step; hook the
                # tracker early so offline benchmarks are not dropped.
                try:
                    _ensure_tracker_hook(self)
                except Exception:
                    pass
                return await run_initial(self, *args, **kwargs)

            run_initial_wrapper._probing_rl_hook = True
            _ORIGINAL_RUN_INITIAL_EVALUATE = run_initial
            trainer_cls._run_initial_evaluate = run_initial_wrapper
            _INSTALLED_HOOKS.append("_run_initial_evaluate")

        if not _INSTALLED_HOOKS:
            _record_install_failure(
                f"{trainer_cls.__name__} exposes none of {', '.join(_HOOK_POINTS)}"
            )
            return False
        _ENABLED = True
        return True
    except Exception as exc:
        _record_install_failure(f"{type(exc).__name__}: {exc}")
        return False


def init() -> None:
    """Extension entry point."""

    install()


def deinit() -> None:
    """Disable collection and restore the XTuner trainer method."""

    global _ENABLED, _ORIGINAL_LOG_STEP, _ORIGINAL_SAVE_TRAJECTORIES
    global _ORIGINAL_RUN_INITIAL_EVALUATE
    _ENABLED = False
    if _ORIGINAL_LOG_STEP is None and _ORIGINAL_RUN_INITIAL_EVALUATE is None:
        return
    try:
        trainer_cls = _resolve_rl_trainer_cls()
        if trainer_cls is not None:
            if _ORIGINAL_LOG_STEP is not None:
                trainer_cls._log_step = _ORIGINAL_LOG_STEP
            if _ORIGINAL_SAVE_TRAJECTORIES is not None:
                trainer_cls._save_trajectories = _ORIGINAL_SAVE_TRAJECTORIES
            if _ORIGINAL_RUN_INITIAL_EVALUATE is not None:
                trainer_cls._run_initial_evaluate = _ORIGINAL_RUN_INITIAL_EVALUATE
    except Exception:
        pass
    _ORIGINAL_LOG_STEP = None
    _ORIGINAL_SAVE_TRAJECTORIES = None
    _ORIGINAL_RUN_INITIAL_EVALUATE = None


__all__ = [
    "XtunerAdapter",
    "canonicalize_scalars",
    "deinit",
    "framework",
    "init",
    "install",
    "status",
]

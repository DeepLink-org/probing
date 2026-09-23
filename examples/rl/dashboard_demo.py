"""Generate live framework-neutral RL telemetry for the Probing Web UI."""

from __future__ import annotations

import argparse
import dataclasses
import math
import random
import time
from typing import Optional

from probing import rl
from probing.ext import rl_data
from probing.ext.rl_data.writer import RlTelemetryWriter


# (data source, category) pairs so the composition panel has a real hierarchy:
# several sources roll up into each category.
_DRIFT: dict[tuple[str, str], float] = {}


def _drift(run_id: str, key: str, low: float, high: float, rng: random.Random) -> float:
    """A value that wanders inside `[low, high]` instead of resampling each step.

    Independent samples, and the `step % n` sawtooths this replaced, draw as a
    solid band once a chart covers more than a couple hundred steps. Real
    telemetry is autocorrelated, so the demo should be too.
    """

    slot = (run_id, key)
    span = high - low
    current = _DRIFT.get(slot, low + span * 0.5)
    current += (rng.random() - 0.5) * span * 0.18
    current = min(high, max(low, current))
    _DRIFT[slot] = current
    return current


def _learning_curve(
    step: int, start: float, ceiling: float, halflife: int, offset: float
) -> float:
    """Approach `ceiling` asymptotically so long runs keep a readable slope.

    A linear ramp clipped at a maximum flattens into a useless straight line
    once it saturates, which is what the demo used to do past ~step 170.
    """

    progress = 1.0 - math.exp(-step / max(halflife, 1))
    return start + (ceiling - start) * progress + offset


# Responses sampled per prompt, the `n` in `batch x n`.
GROUP_SIZE = 16

TASKS = [
    ("openai/gsm8k", "reasoning"),
    ("competition/aime", "reasoning"),
    ("livecodebench/python", "code"),
    ("swebench/verified", "code"),
    ("internal/tool_use", "agent"),
    ("webarena/shopping", "agent"),
    ("internal/chat_rlhf", "chat"),
    ("safety/redteam", "safety"),
]


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--run-id", default="probing-rl-demo")
    parser.add_argument(
        "--secondary-run-id",
        default="probing-rl-demo-b",
        help="Optional second run for selector testing; empty disables it",
    )
    parser.add_argument("--interval", type=float, default=1.0)
    parser.add_argument("--steps", type=int, default=10_000)
    parser.add_argument(
        "--notice-step",
        type=int,
        default=3,
        help="Step at which to file a demo operator notice",
    )
    return parser.parse_args()


PROMPTS_PER_STEP = 6
ROLLOUTS_PER_PROMPT = 8


@dataclasses.dataclass
class _Prompt:
    """One sampled prompt and how each of its rollouts turned out.

    A rollout outcome is `True` when the judge passed it, `False` when a filter
    removed it, and `None` when it was lost to an infrastructure failure.
    """

    slot: int
    task_index: int
    group_id: str
    rollouts: list[Optional[bool]]

    @property
    def judged(self) -> int:
        return sum(1 for outcome in self.rollouts if outcome is not None)

    @property
    def pass_rate(self) -> float:
        """Share of judged rollouts that passed, or 0 when none were judged."""

        judged = self.judged
        if judged == 0:
            return 0.0
        return sum(1 for outcome in self.rollouts if outcome) / judged


def _sample_prompts(
    run_id: str, step: int, rng: random.Random, offset: float
) -> list[_Prompt]:
    """Roll out several prompts, with per-rollout pass odds set by the curve.

    Deriving the pass gauges from these outcomes keeps them consistent with the
    per-sample rows, which independent formulas for each could not do: the
    histogram would disagree with the `pass_zero`/`pass_one` gauges above it.
    """

    pass_odds = min(0.97, max(0.02, _learning_curve(step, 0.25, 0.88, 700, offset)))
    prompts = []
    for slot in range(PROMPTS_PER_STEP):
        rollouts: list[Optional[bool]] = []
        for _ in range(ROLLOUTS_PER_PROMPT):
            if rng.random() < 0.01:
                rollouts.append(None)  # infrastructure failure
            elif rng.random() < pass_odds:
                rollouts.append(True)
            else:
                rollouts.append(False)
        prompts.append(
            _Prompt(
                slot=slot,
                task_index=step * PROMPTS_PER_STEP + slot,
                group_id=f"group-{step:05d}-{slot:02d}",
                rollouts=rollouts,
            )
        )
    return prompts


def _share(prompts: list[_Prompt], predicate) -> float:
    if not prompts:
        return 0.0
    return sum(1 for prompt in prompts if predicate(prompt.pass_rate)) / len(prompts)


def _mean_pass_rate(prompts: list[_Prompt]) -> float:
    if not prompts:
        return 0.0
    return sum(prompt.pass_rate for prompt in prompts) / len(prompts)


def emit_step(
    writer: RlTelemetryWriter,
    *,
    run_id: str,
    step: int,
    started: int,
    samples_total: int,
    tokens_total: int,
    rng: random.Random,
    offset: float,
    total_steps: int,
) -> tuple[int, int]:
    # A step samples several prompts, each with a group of rollouts, because the
    # pass-rate spread across prompts is only meaningful with more than one.
    prompts = _sample_prompts(run_id, step, rng, offset)
    accepted = sum(len(prompt.rollouts) for prompt in prompts)
    failed = sum(
        1 for prompt in prompts for outcome in prompt.rollouts if outcome is None
    )
    filtered = sum(
        1 for prompt in prompts for outcome in prompt.rollouts if outcome is False
    )
    trained = sum(
        1 for prompt in prompts for outcome in prompt.rollouts if outcome is True
    )
    # Responses lengthen as the policy learns to reason, with step-to-step jitter.
    response_tokens = int(
        _learning_curve(step, 420, 1100, 900, 0)
        + _drift(run_id, "resp_tok", -60.0, 60.0, rng)
    )
    step_tokens = trained * (256 + response_tokens)
    samples_total += trained
    tokens_total += step_tokens
    step_seconds = _drift(run_id, "time.step_s", 12.0, 16.0, rng)
    rollout_seconds = _drift(run_id, "time.rollout_s", 7.0, 9.0, rng)
    task_p50 = rollout_seconds / max(accepted, 1)
    # One ratio for both the p99 and the reported p99/p50, which otherwise
    # disagree with each other.
    tail_ratio = _drift(run_id, "task_p99_p50_ratio", 1.2, 1.8, rng)

    writer.write_run(
        phase="training" if step % 4 else "rollout",
        global_step=step,
        start_time_ns=started,
        samples_total=samples_total,
        tokens_total=tokens_total,
    )
    writer.append_sampler(
        step,
        {
            "target": 10,
            "accepted": accepted,
            "judged": accepted - failed,
            "trained": trained,
            "filtered": filtered,
            "failed": failed,
            "expired": 1 if step % 13 == 0 else 0,
            "in_flight": round(_drift(run_id, "in_flight", 2.0, 5.0, rng)),
        },
    )
    writer.append_metrics(
        step,
        {
            "progress.total_steps": float(total_steps),
            "progress.completed_steps": float(step),
            "progress.completed_ratio": min(1.0, step / max(total_steps, 1)),
            "progress.eta_s": max(0, total_steps - step) * step_seconds,
            "sampler.avg_pass": _mean_pass_rate(prompts),
            "reward.mean": _learning_curve(step, 0.2, 0.80, 800, offset)
            + _drift(run_id, "reward.mean", 0.0, 0.04, rng),
            "reward.raw_mean": 0.2 + step * 0.003 + offset,
            "reward.min": 0.0,
            "reward.max": 1.0,
            "advantage.mean": _drift(run_id, "advantage.mean", -0.025, 0.025, rng),
            "advantage.min": -1.5,
            "advantage.max": 1.5,
            # Every trend here is spent well before a long run ends, so the noise on
            # top has to drift too. Independent noise leaves the rest of the run as
            # nothing but jitter between the trend's floor and the noise amplitude.
            "policy.entropy": _learning_curve(step, 1.70, 1.02, 420, 0)
            + _drift(run_id, "policy.entropy", 0.0, 0.02, rng),
            "policy.entropy_rollout": _learning_curve(step, 1.72, 1.04, 420, 0)
            + _drift(run_id, "policy.entropy_rollout", 0.0, 0.02, rng),
            "policy.pg_loss": 0.18 * math.exp(-step / 80)
            + _drift(run_id, "policy.pg_loss", 0.0, 0.01, rng),
            "policy.base_loss": 0.22 * math.exp(-step / 90)
            + _drift(run_id, "policy.base_loss", 0.0, 0.01, rng),
            "policy.grad_norm": _drift(run_id, "policy.grad_norm", 0.7, 1.0, rng),
            "policy.clip_frac_high": _learning_curve(step, 0.005, 0.075, 500, 0)
            + _drift(run_id, "policy.clip_frac_high", 0.0, 0.008, rng),
            "policy.clip_frac_low": _learning_curve(step, 0.002, 0.045, 500, 0)
            + _drift(run_id, "policy.clip_frac_low", 0.0, 0.006, rng),
            "policy.kl1": _drift(run_id, "policy.kl1", 0.004, 0.006, rng),
            "policy.kl3": _drift(run_id, "policy.kl3", 0.006, 0.009, rng),
            "policy.ratio_max": _drift(run_id, "policy.ratio_max", 1.05, 1.20, rng),
            "policy.ratio_min": _drift(run_id, "policy.ratio_min", 0.80, 0.95, rng),
            "policy.ratio_abs_dev_mean": _drift(
                run_id, "policy.ratio_abs_dev_mean", 0.01, 0.02, rng
            ),
            "policy.train_infer_kl": _drift(
                run_id, "policy.train_infer_kl", 0.008, 0.012, rng
            ),
            "policy.train_infer_k3_kl": _drift(
                run_id, "policy.train_infer_k3_kl", 0.012, 0.017, rng
            ),
            "policy.train_infer_ppl_ratio": _drift(
                run_id, "policy.train_infer_ppl_ratio", 1.0, 1.02, rng
            ),
            "context.prompt_tokens.mean": 256.0,
            "context.response_tokens.mean": float(response_tokens),
            "context.response_tokens.max": float(response_tokens + 120),
            "context.response_tokens.std": _drift(
                run_id, "context.response_tokens.std", 60.0, 100.0, rng
            ),
            "context.total_tokens.mean": 256 + response_tokens,
            "agent.turns.mean": _drift(run_id, "turns", 2.0, 5.0, rng),
            "train.tokens": step_tokens,
            "train.seqlen_tokens": float(step_tokens + 512),
            "train.efficient_attn_ratio": _drift(
                run_id, "train.efficient_attn_ratio", 0.1, 0.15, rng
            ),
            "time.step_s": step_seconds,
            "time.rollout_s": rollout_seconds,
            "time.training_s": _drift(run_id, "time.training_s", 4.0, 5.0, rng),
            "time.onload_s": _drift(run_id, "time.onload_s", 0.2, 0.3, rng),
            "time.offload_s": _drift(run_id, "time.offload_s", 0.07, 0.11, rng),
            "time.switch_to_rollout_s": _drift(
                run_id, "time.switch_to_rollout_s", 0.45, 0.55, rng
            ),
            # Checkpointing really is periodic, so the step test stays.
            "time.save_ckpt_s": _drift(run_id, "time.save_ckpt_s", 0.5, 0.7, rng)
            if step % 5 == 0
            else 0.0,
            "time.prepare_data_s": _drift(
                run_id, "time.prepare_data_s", 0.015, 0.025, rng
            ),
            "throughput.e2e_samples_s": trained / step_seconds,
            "throughput.e2e_tokens_s": step_tokens / step_seconds,
            "throughput.effective_tokens_s": step_tokens / max(step_seconds - 0.5, 0.1),
            "throughput.training_tokens_s": step_tokens
            / max(_drift(run_id, "training_tokens_s.div", 4.0, 5.0, rng), 0.1),
            "throughput.rollout_samples_s": accepted / rollout_seconds,
            "throughput.rollout_tokens_s": (accepted * response_tokens)
            / rollout_seconds,
            "sampler.pass_zero_ratio": _share(prompts, lambda rate: rate <= 0.0),
            "sampler.pass_one_ratio": _share(prompts, lambda rate: rate >= 1.0),
            "sampler.infra_error_ratio": failed / max(accepted, 1),
            "sampler.batch_size": float(accepted),
            "sampler.group_size": float(GROUP_SIZE),
            "sampler.task_count": float(accepted),
            "sampler.task_mean_s": task_p50
            * _drift(run_id, "task_mean_s.factor", 1.0, 1.05, rng),
            "sampler.task_p50_s": task_p50,
            "sampler.task_p99_s": task_p50 * tail_ratio,
            "sampler.task_p99_p50_ratio": tail_ratio,
            "hardware.max_memory_gb": _drift(run_id, "max_memory_gb", 58.0, 64.0, rng),
            "hardware.reserved_memory_gb": _drift(
                run_id, "reserved_memory_gb", 68.0, 72.0, rng
            ),
            "environment.active": round(_drift(run_id, "env_active", 6.0, 8.0, rng)),
            "environment.sandbox_total": float(1200 + step * accepted),
            "environment.setup_error_ratio": max(
                0.0,
                0.12
                - step * 0.001
                + _drift(run_id, "setup_error_ratio", 0.0, 0.02, rng),
            ),
            "environment.queue_s": _drift(run_id, "environment.queue_s", 0.4, 1.1, rng),
            # Leaks are occasional, not every seventh step.
            "environment.leak_count": float(rng.random() < 0.08),
            "sampler.avg_staleness": _drift(run_id, "staleness", 0.0, 2.0, rng),
            "sampler.measurable_prompts": accepted - failed,
            "cost.usd_total": 120.0 + step * 18.5 + offset * 40,
            "cost.usd_per_hour": 640.0 + offset * 80,
            "hardware.restart_count": 0.0 if step < 50 else 1.0 + (step // 80),
            "about.restarts": 0.0 if step < 50 else 1.0 + (step // 80),
            "about.target_steps": float(total_steps),
            "about.progress": min(1.0, step / max(total_steps, 1)),
        },
    )

    index = 0
    for prompt in prompts:
        task, category = TASKS[prompt.task_index % len(TASKS)]
        for slot, outcome in enumerate(prompt.rollouts):
            rollout_id = f"{run_id}-{step:05d}-{prompt.slot:02d}-{slot:02d}"
            if outcome is None:
                status = "failed"
            elif outcome:
                status = "completed"
            else:
                status = "filtered"
            # Spread staleness across 0/1/2/3/4+ buckets for histogram demos.
            staleness = [0, 1, 2, 3, 5, 0, 1, 4][index % 8]
            with rl.context(
                run_id=run_id,
                framework="demo",
                step_id=step,
                rollout_id=rollout_id,
                group_id=prompt.group_id,
                sample_id=rollout_id,
            ):
                with rl.span("rollout.generate"):
                    time.sleep(0.0002)
                with rl.span("judger.run"):
                    time.sleep(0.0001)
            writer.append_sample(
                {
                    "step": step,
                    "rollout_id": rollout_id,
                    "group_id": prompt.group_id,
                    "task": task,
                    "category": category,
                    "status": status,
                    "reward": {
                        "score": 1.0 if outcome else 0.0,
                        "pass": bool(outcome),
                    },
                    "filter_reason": "duplicate" if status == "filtered" else "",
                    "prompt_tokens": 256,
                    "response_tokens": response_tokens + slot * 3,
                    "staleness": staleness,
                }
            )
            index += 1

    if step % 5 == 0:
        writer.append_benchmark(
            "math@hard",
            step,
            _learning_curve(step, 0.35, 0.84, 700, offset),
            sample_count=64,
            version="v1.1",
            harness="demo-math-agent",
            aggregation="avg@3",
        )
        writer.append_benchmark(
            "code@live",
            step,
            min(0.9, 0.28 + step * 0.0035 + offset / 2),
            sample_count=48,
            version="v0.9",
            harness="demo-code-agent",
            aggregation="pass@1",
        )

    return samples_total, tokens_total


def main() -> None:
    args = parse_args()
    rng = random.Random(7)
    started = time.time_ns()
    rl_data.init()
    writer = RlTelemetryWriter("demo", args.run_id, rank=0, source="dashboard_demo")
    writer.bind_run(
        args.run_id,
        phase="initializing",
        start_time_ns=started,
        samples_total=0,
        tokens_total=0,
        job_id=f"job-{args.run_id}",
        config_hash="demo-cfg-a1b2c3",
        metadata={
            "model": "demo-7b",
            "lr": 1.0e-6,
            "rollout_batch": 64,
            "algo": "grpo",
            "dataset": "math+code+agent",
        },
    )
    secondary = None
    secondary_started = started
    secondary_samples = 0
    secondary_tokens = 0
    if args.secondary_run_id:
        secondary = RlTelemetryWriter(
            "demo", args.secondary_run_id, rank=0, source="dashboard_demo"
        )
        secondary.bind_run(
            args.secondary_run_id,
            phase="initializing",
            start_time_ns=secondary_started,
            samples_total=0,
            tokens_total=0,
            job_id=f"job-{args.secondary_run_id}",
            config_hash="demo-cfg-d4e5f6",
            metadata={
                "model": "demo-7b",
                "lr": 8.0e-7,
                "rollout_batch": 48,
                "algo": "grpo",
                "dataset": "math+code",
            },
        )

    samples_total = 0
    tokens_total = 0
    for step in range(1, args.steps + 1):
        samples_total, tokens_total = emit_step(
            writer,
            run_id=args.run_id,
            step=step,
            started=started,
            samples_total=samples_total,
            tokens_total=tokens_total,
            rng=rng,
            offset=0.0,
            total_steps=args.steps,
        )
        if step == args.notice_step:
            writer.append_notice(
                "Restarted after a node drain; rollout weights were resynced.",
                step=step,
                level="warning",
                kind="restart",
                author="oncall",
            )
        if secondary is not None and step % 2 == 0:
            secondary_samples, secondary_tokens = emit_step(
                secondary,
                run_id=args.secondary_run_id,
                step=step // 2,
                started=secondary_started,
                samples_total=secondary_samples,
                tokens_total=secondary_tokens,
                rng=rng,
                offset=-0.05,
                total_steps=args.steps // 2,
            )

        print(
            f"step={step} reward={0.2 + step * 0.003:.3f} "
            f"trained_tokens={tokens_total}",
            flush=True,
        )
        time.sleep(max(0.05, args.interval))

    writer.write_run(
        phase="ended",
        global_step=args.steps,
        start_time_ns=started,
        end_time_ns=time.time_ns(),
        samples_total=samples_total,
        tokens_total=tokens_total,
    )
    if secondary is not None:
        secondary.write_run(
            phase="ended",
            global_step=args.steps // 2,
            start_time_ns=secondary_started,
            end_time_ns=time.time_ns(),
            samples_total=secondary_samples,
            tokens_total=secondary_tokens,
        )


if __name__ == "__main__":
    main()

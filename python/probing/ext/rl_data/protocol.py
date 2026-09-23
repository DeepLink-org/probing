"""Stable adapter contract and canonical RL metric names."""

from __future__ import annotations

from typing import Any, Mapping, Protocol, runtime_checkable


CANONICAL_METRICS = {
    "progress.total_steps": "Planned training steps for the whole run",
    "progress.completed_steps": "Training steps finished so far",
    "progress.completed_ratio": "Finished share of the planned training steps",
    "progress.eta_s": "Projected seconds remaining at the recent step rate",
    "reward.mean": "Mean reward over trajectories trained in one step",
    "reward.raw_mean": "Mean reward before shaping or normalization",
    "reward.min": "Lowest reward in the trained batch",
    "reward.max": "Highest reward in the trained batch",
    "advantage.mean": "Mean advantage over the trained batch",
    "advantage.min": "Lowest advantage in the trained batch",
    "advantage.max": "Highest advantage in the trained batch",
    "policy.entropy": "Mean per-token policy entropy",
    "policy.entropy_rollout": "Mean per-token entropy as measured by the sampler",
    "policy.pg_loss": "Policy-gradient loss",
    "policy.base_loss": "Unweighted language-model loss before RL terms",
    "policy.grad_norm": "Global gradient norm before clipping",
    "policy.clip_frac_high": "Share of tokens clipped at the upper ratio bound",
    "policy.clip_frac_low": "Share of tokens clipped at the lower ratio bound",
    "policy.kl1": "k1 estimator of KL against the reference policy",
    "policy.kl3": "k3 estimator of KL against the reference policy",
    "policy.ratio_max": "Largest importance ratio in the trained batch",
    "policy.ratio_min": "Smallest importance ratio in the trained batch",
    "policy.ratio_abs_dev_mean": "Mean absolute deviation of importance ratios from 1",
    "policy.train_infer_kl": "KL between rollout and trainer token distributions",
    "policy.train_infer_k3_kl": "k3 estimator of the rollout-versus-trainer KL",
    "policy.train_infer_logprob_abs_diff": (
        "Mean absolute per-token logprob gap between sampler and trainer"
    ),
    "policy.train_infer_log_ppl_diff": "Log-perplexity gap between sampler and trainer",
    "policy.train_infer_log_ppl_abs_diff": (
        "Absolute log-perplexity gap between sampler and trainer"
    ),
    "policy.train_infer_log_ppl_diff_max": "Largest observed log-perplexity gap",
    "policy.train_infer_log_ppl_diff_min": "Smallest observed log-perplexity gap",
    "policy.train_infer_ppl_ratio": "Trainer perplexity divided by sampler perplexity",
    "policy.rollout_ppl": "Sequence perplexity as scored by the sampler",
    "policy.rollout_log_ppl": "Sequence log-perplexity as scored by the sampler",
    "policy.training_ppl": "Sequence perplexity as scored by the trainer",
    "policy.training_log_ppl": "Sequence log-perplexity as scored by the trainer",
    "context.prompt_tokens.mean": "Mean prompt token count per trajectory",
    "context.prompt_tokens.min": "Shortest prompt in the batch, in tokens",
    "context.prompt_tokens.max": "Longest prompt in the batch, in tokens",
    "context.response_tokens.mean": "Mean generated token count per trajectory",
    "context.response_tokens.min": "Shortest response in the batch, in tokens",
    "context.response_tokens.max": "Longest response in the batch, in tokens",
    "context.response_tokens.std": "Standard deviation of response length in tokens",
    "context.total_tokens.mean": "Mean prompt plus response token count",
    "agent.turns.mean": "Mean agent turns per trajectory",
    "train.tokens": "Tokens trained in one step",
    "train.img_tokens": "Image tokens trained in one step",
    "train.seqlen_tokens": "Padded sequence capacity consumed in one step",
    "train.efficient_attn_ratio": "Share of attention compute spent on real tokens",
    "train.img_efficient_attn_ratio": "Image-token share of efficient attention compute",
    "time.step_s": "Whole-step wall-clock duration in seconds",
    "time.rollout_s": "Rollout generation wall-clock duration in seconds",
    "time.training_s": "Policy training wall-clock duration in seconds",
    "time.onload_s": "Seconds spent loading weights onto the accelerator",
    "time.offload_s": "Seconds spent offloading weights off the accelerator",
    "time.switch_to_rollout_s": "Seconds spent handing weights to the sampler",
    "time.save_ckpt_s": "Seconds spent writing a checkpoint",
    "time.prepare_data_s": "Seconds spent preparing the training batch",
    "sampler.avg_pass": "Mean success fraction across prompt attempt groups",
    "sampler.pass_zero_ratio": "Share of prompt groups with no successful attempt",
    "sampler.pass_one_ratio": "Share of prompt groups with every attempt successful",
    "sampler.infra_error_ratio": "Share of attempts lost to infrastructure failures",
    "sampler.measurable_prompts": "Prompt groups with a measurable pass rate",
    "sampler.avg_staleness": "Mean policy-version distance at training time",
    "sampler.batch_size": "Trajectories accepted into the trained batch",
    "sampler.group_size": "Responses sampled per prompt in the trained batch",
    "sampler.task_count": "Rollout tasks timed in the step",
    "sampler.task_mean_s": "Mean rollout task duration in seconds",
    "sampler.task_p50_s": "Median rollout task duration in seconds",
    "sampler.task_p99_s": "99th-percentile rollout task duration in seconds",
    "sampler.task_p99_p50_ratio": "Straggler spread, as p99 over p50 task duration",
    "environment.active": "Active sandbox or environment count",
    "environment.sandbox_total": "Cumulative sandbox or environment executions",
    "environment.setup_error_ratio": "Share of environment setups that failed",
    "environment.queue_s": "Mean environment queue wait in seconds",
    "environment.leak_count": "Environments still open after sample completion",
    "throughput.e2e_samples_s": "End-to-end trajectories trained per second",
    "throughput.e2e_tokens_s": "End-to-end tokens trained per second",
    "throughput.effective_samples_s": "Trajectories per second excluding idle time",
    "throughput.effective_tokens_s": "Tokens per second excluding idle time",
    "throughput.training_tokens_s": "Tokens per second during the training phase",
    "throughput.rollout_samples_s": "Trajectories per second during rollout",
    "throughput.rollout_tokens_s": "Tokens per second during rollout",
    "cost.usd_total": "Cumulative reported compute spend in USD",
    "cost.usd_per_hour": "Recent compute spend rate in USD per hour",
    "hardware.restart_count": "Cumulative hardware or node restart count",
    "hardware.max_memory_gb": "Peak accelerator memory allocated, in GiB",
    "hardware.reserved_memory_gb": "Accelerator memory reserved by the allocator, in GiB",
}


@runtime_checkable
class RlFrameworkAdapter(Protocol):
    """Contract implemented by framework-specific RL telemetry adapters."""

    framework: str

    def bind_run(self, run_id: str, **metadata: Any) -> None:
        """Bind the adapter to a run and emit its initial state."""

    def on_scalars(
        self,
        step: int,
        scalars: Mapping[str, Any],
        **context: Any,
    ) -> None:
        """Consume one trainer scalar batch."""

    def on_sampler_snapshot(
        self,
        step: int,
        counts: Mapping[str, Any],
        **context: Any,
    ) -> None:
        """Consume one sampling-pipeline snapshot."""

    def on_sample(self, sample: Mapping[str, Any], **context: Any) -> None:
        """Consume one rollout sample outcome."""

    def on_benchmark(
        self,
        name: str,
        step: int,
        score: float,
        **context: Any,
    ) -> None:
        """Consume one benchmark result."""


__all__ = ["CANONICAL_METRICS", "RlFrameworkAdapter"]

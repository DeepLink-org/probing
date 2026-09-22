from __future__ import annotations

import sys
import types
from types import SimpleNamespace

from probing.ext.rl_data.protocol import RlFrameworkAdapter
from probing.ext.xtuner import XtunerAdapter, canonicalize_scalars, deinit, install


def test_canonicalize_xtuner_scalars():
    result = canonicalize_scalars(
        {
            "response/rewards/mean": 0.5,
            "entropy/train": 1.25,
            "response/prompt_len/mean": 10,
            "response/response_len/mean": 6,
            "time/step": 4.0,
            "throughput/e2e_effective_tgs": 275.0,
            "throughput/training_tgs": 559.0,
            "throughput/rollout_tgs": 1862.0,
            "train_metrics/worker_0/step_avg_grad_norm": 2.0,
            "train_metrics/worker_0/reduced_llm_loss": 0.3,
            "train_metrics/worker_0/step_avg_reduced_llm_loss": 0.25,
            "train_metrics/worker_0/step_consumed_tokens": 100,
            "train_metrics/worker_0/step_avg_step_consumed_tokens": 128,
            "mismatch/mismatch_kl": 0.01,
        }
    )

    assert result["reward.mean"] == 0.5
    assert result["policy.entropy"] == 1.25
    assert result["context.total_tokens.mean"] == 16.0
    assert result["policy.grad_norm"] == 2.0
    assert result["policy.pg_loss"] == 0.25
    assert result["train.tokens"] == 128
    assert result["policy.train_infer_kl"] == 0.01
    assert result["time.step_s"] == 4.0
    assert result["throughput.e2e_tokens_s"] == 275.0
    assert result["throughput.training_tokens_s"] == 559.0
    assert result["throughput.rollout_tokens_s"] == 1862.0


def test_canonicalize_covers_clip_advantage_and_memory_families():
    result = canonicalize_scalars(
        {
            "response/advantages/mean": -0.014,
            "response/advantages/max": 1.5,
            "response/raw_rewards/mean": 0.28,
            "response/response_len/std": 207.35,
            "response/batch_size": 32,
            "entropy/rollout": 0.515,
            "mismatch/mismatch_k3_kl": 0.0007,
            "mismatch/mismatch_ppl_ratio": 1.0004,
            "time/save_ckpt": 0.48,
            "timing/task_p99_p50_ratio": 1.003,
            "throughput/effective_tgs": 913.7,
            "train_metrics/worker_0/local_base_loss": 0.033,
            "train_metrics/worker_0/max_memory": 3.48,
            "train_metrics/worker_0/reserved_memory": 4.66,
            "train_metrics/worker_0/efficient_attn_ratio": 0.111,
            "train_metrics/worker_0/img_efficient_attn_ratio": 0.0,
            "train_metrics/worker_0/reduced_train_policy_clip_frac_high": 0.02,
            "train_metrics/worker_0/reduced_train_policy_kl3": 0.004,
            "train_metrics/worker_0/reduced_train_policy_ratio_max": 1.2,
            "train_metrics/worker_0/step_avg_step_seqlen_tokens": 8192.0,
        }
    )

    assert result["advantage.mean"] == -0.014
    assert result["advantage.max"] == 1.5
    assert result["reward.raw_mean"] == 0.28
    assert result["context.response_tokens.std"] == 207.35
    assert result["sampler.batch_size"] == 32
    assert result["policy.entropy_rollout"] == 0.515
    assert result["policy.train_infer_k3_kl"] == 0.0007
    assert result["policy.train_infer_ppl_ratio"] == 1.0004
    assert result["time.save_ckpt_s"] == 0.48
    assert result["sampler.task_p99_p50_ratio"] == 1.003
    assert result["throughput.effective_tokens_s"] == 913.7
    assert result["policy.clip_frac_high"] == 0.02
    assert result["policy.kl3"] == 0.004
    assert result["policy.ratio_max"] == 1.2
    assert result["train.seqlen_tokens"] == 8192.0
    assert result["hardware.max_memory_gb"] == 3.48
    assert result["hardware.reserved_memory_gb"] == 4.66
    # The longest matching suffix wins, so these stay distinct.
    assert result["policy.base_loss"] == 0.033
    assert result["train.efficient_attn_ratio"] == 0.111
    assert result["train.img_efficient_attn_ratio"] == 0.0


def test_progress_metrics_report_completion_and_eta():
    class Writer:
        def __init__(self):
            self.metrics = []
            self.runs = []

        def append_metrics(self, step, metrics, **kwargs):
            self.metrics.append((step, metrics))

        def append_benchmark(self, *args, **kwargs):
            pass

        def write_run(self, **kwargs):
            self.runs.append(kwargs)

    writer = Writer()
    adapter = XtunerAdapter(writer)
    adapter.total_steps = 100
    adapter.on_scalars(25, {"time/step": 8.0})

    _, metrics = writer.metrics[-1]
    assert metrics["progress.total_steps"] == 100.0
    assert metrics["progress.completed_steps"] == 25.0
    assert metrics["progress.completed_ratio"] == 0.25
    assert metrics["progress.eta_s"] == 600.0


def test_progress_metrics_absent_without_total_steps():
    class Writer:
        def __init__(self):
            self.metrics = []

        def append_metrics(self, step, metrics, **kwargs):
            self.metrics.append((step, metrics))

        def append_benchmark(self, *args, **kwargs):
            pass

        def write_run(self, **kwargs):
            pass

    writer = Writer()
    adapter = XtunerAdapter(writer)
    adapter.on_scalars(25, {"time/step": 8.0})

    _, metrics = writer.metrics[-1]
    assert not any(name.startswith("progress.") for name in metrics)


def test_adapter_matches_protocol():
    class Writer:
        pass

    assert isinstance(XtunerAdapter(Writer()), RlFrameworkAdapter)


def test_samples_emit_outcomes_and_group_metrics():
    class Writer:
        def __init__(self):
            self.samples = []
            self.metrics = []
            self.sampler = []
            self.benchmarks = []
            self.samples_total = 0
            self.tokens_total = 0
            self.runs = []

        def append_sample(self, sample, **_kwargs):
            self.samples.append(sample)

        def append_metrics(self, step, values, **_kwargs):
            self.metrics.append((step, values))

        def bump_totals(self, *, samples=0, tokens=0):
            self.samples_total += samples
            self.tokens_total += tokens

        def write_run(self, **kwargs):
            self.runs.append(kwargs)

        def append_sampler(self, step, counts, **_kwargs):
            self.sampler.append((step, dict(counts)))

        def append_benchmark(self, name, step, score, **kwargs):
            self.benchmarks.append((name, step, score, kwargs))

    writer = Writer()
    adapter = XtunerAdapter(writer)
    samples = [
        SimpleNamespace(
            rollout_id="r1",
            group_id="g1",
            uid="s1",
            task_name=None,
            data_source={"openai/gsm8k": 1.0},
            status=SimpleNamespace(value="completed"),
            reward={"score": 1, "pass": True},
            prompt_ids=[1, 2],
            response_ids=[3],
            seq_staleness=1,
            extra_fields={},
        ),
        SimpleNamespace(
            rollout_id="r2",
            group_id="g1",
            uid="s2",
            task_name=None,
            data_source={"openai/gsm8k": 1.0},
            status=SimpleNamespace(value="completed"),
            reward={"score": 0, "pass": False},
            prompt_ids=[1],
            response_ids=[2, 3],
            seq_staleness=3,
            extra_fields={"filter_reason": "duplicate"},
        ),
    ]

    adapter.on_samples(5, samples)

    assert len(writer.samples) == 2
    assert writer.samples[0]["task_name"] == "openai/gsm8k"
    assert writer.samples[1]["filter_reason"] == "duplicate"
    derived = writer.metrics[0][1]
    assert derived["sampler.avg_pass"] == 0.5
    assert derived["sampler.pass_zero_ratio"] == 0.0
    assert derived["sampler.pass_one_ratio"] == 0.0
    assert derived["sampler.avg_staleness"] == 2.0
    assert derived["train.tokens"] == 6.0
    assert derived["environment.active"] == 2.0
    assert derived["environment.sandbox_total"] == 2.0
    assert writer.samples_total == 2
    assert writer.tokens_total == 6
    assert writer.sampler[-1][1]["accepted"] == 2
    assert writer.sampler[-1][1]["trained"] == 2
    assert writer.benchmarks[0][0] == "online/avg_pass"
    assert writer.benchmarks[0][2] == 0.5


def test_eval_scalars_emit_benchmarks():
    class Writer:
        def __init__(self):
            self.metrics = []
            self.benchmarks = []
            self.tokens_total = 0
            self.samples_total = 0

        def append_metrics(self, step, values, **_kwargs):
            self.metrics.append((step, dict(values)))

        def bump_totals(self, **_kwargs):
            return None

        def write_run(self, **_kwargs):
            return True

        def append_sampler(self, *args, **kwargs):
            return True

        def append_benchmark(self, name, step, score, **kwargs):
            self.benchmarks.append((name, step, score, kwargs))

    adapter = XtunerAdapter(Writer())
    adapter.on_scalars(
        0,
        {
            "eval/avg_pass@1": 0.42,
            "eval/score": 0.55,
            "response/rewards/mean": 0.1,
        },
    )
    names = {item[0] for item in adapter.writer.benchmarks}
    assert names == {"avg_pass@1", "score"}
    assert all(
        item[3].get("version") == "xtuner-eval" for item in adapter.writer.benchmarks
    )


def test_install_hooks_existing_tracker(monkeypatch):
    captured = []

    class Tracker:
        def add_scalars(self, tag_scalar_dict, global_step):
            captured.append(("original", global_step, tag_scalar_dict))

    class Trainer:
        def __init__(self):
            self._exp_tracker = Tracker()
            self.exp_dir = "/tmp/run-1"

        def _log_step(self, step):
            self._exp_tracker.add_scalars(
                tag_scalar_dict={"response/rewards/mean": 0.75},
                global_step=step,
            )

    writer_methods = {
        "__init__": lambda self, framework, run_id, **kwargs: (
            (
                setattr(self, "framework", framework),
                setattr(self, "run_id", run_id),
                setattr(self, "tokens_total", 0),
                setattr(self, "samples_total", 0),
            )
            and None
        ),
        "bind_run": lambda self, *args, **kwargs: None,
        "append_metrics": lambda self, step, values, **kwargs: (
            captured.append(("metrics", step, dict(values))) or len(values)
        ),
        "write_run": lambda self, **kwargs: True,
        "bump_totals": lambda self, **kwargs: None,
        "append_sampler": lambda self, *args, **kwargs: True,
        "append_sample": lambda self, *args, **kwargs: True,
        "append_benchmark": lambda self, *args, **kwargs: True,
    }
    fake_writer = type("FakeWriter", (), writer_methods)

    xtuner_pkg = types.ModuleType("xtuner")
    xtuner_v1 = types.ModuleType("xtuner.v1")
    xtuner_train = types.ModuleType("xtuner.v1.train")
    xtuner_trainer = types.ModuleType("xtuner.v1.train.rl_trainer")
    xtuner_trainer.BaseRLTrainer = Trainer
    xtuner_trainer.RLTrainer = Trainer
    for name, module in {
        "xtuner": xtuner_pkg,
        "xtuner.v1": xtuner_v1,
        "xtuner.v1.train": xtuner_train,
        "xtuner.v1.train.rl_trainer": xtuner_trainer,
    }.items():
        monkeypatch.setitem(sys.modules, name, module)

    import probing.ext.xtuner as adapter_module

    deinit()
    monkeypatch.setattr(adapter_module, "RlTelemetryWriter", fake_writer)
    assert install()

    trainer = Trainer()
    trainer._log_step(3)

    assert captured[0][0] == "original"
    raw_rows = [item for item in captured if item[0] == "metrics"]
    assert raw_rows[0][2]["response/rewards/mean"] == 0.75
    assert raw_rows[1][2]["reward.mean"] == 0.75
    deinit()


def test_group_size_reported_from_trainer_config():
    class Writer:
        def __init__(self):
            self.metrics = []

        def append_metrics(self, step, metrics, **kwargs):
            self.metrics.append((step, metrics))

        def append_benchmark(self, *args, **kwargs):
            pass

        def write_run(self, **kwargs):
            pass

    writer = Writer()
    adapter = XtunerAdapter(writer)
    adapter.group_size = 16.0
    adapter.on_scalars(4, {"response/batch_size": 1568.0})

    _, metrics = writer.metrics[-1]
    assert metrics["sampler.batch_size"] == 1568.0
    assert metrics["sampler.group_size"] == 16.0


def test_group_size_absent_when_unconfigured():
    class Writer:
        def __init__(self):
            self.metrics = []

        def append_metrics(self, step, metrics, **kwargs):
            self.metrics.append((step, metrics))

        def append_benchmark(self, *args, **kwargs):
            pass

        def write_run(self, **kwargs):
            pass

    writer = Writer()
    adapter = XtunerAdapter(writer)
    adapter.on_scalars(4, {"response/batch_size": 1568.0})

    _, metrics = writer.metrics[-1]
    assert "sampler.group_size" not in metrics


def test_sample_mapping_reads_category_from_dataset_fields():
    from probing.ext.xtuner import _sample_mapping

    class State:
        rollout_id = "r-1"
        group_id = "g-1"
        uid = "r-1"
        status = "completed"
        reward = 1.0
        extra_fields = {"data_source": "openai/gsm8k", "domain": "reasoning"}

    mapping = _sample_mapping(State(), 7)
    assert mapping["category"] == "reasoning"

    class Bare(State):
        extra_fields = {"data_source": "openai/gsm8k"}

    # No category key: the writer decides, so the adapter reports nothing.
    assert _sample_mapping(Bare(), 7)["category"] == ""


class _RecordingWriter:
    """Collects what the adapter would have written."""

    def __init__(self):
        self.samples = []
        self.metrics = []
        self.sampler = []
        self.benchmarks = []
        self.samples_total = 0
        self.tokens_total = 0
        self.runs = []

    def append_sample(self, sample, **_kwargs):
        self.samples.append(sample)

    def append_metrics(self, step, values, **_kwargs):
        self.metrics.append((step, values))

    def bump_totals(self, *, samples=0, tokens=0):
        self.samples_total += samples
        self.tokens_total += tokens

    def write_run(self, **kwargs):
        self.runs.append(kwargs)

    def append_sampler(self, step, counts, **_kwargs):
        self.sampler.append((step, dict(counts)))

    def append_benchmark(self, name, step, score, **kwargs):
        self.benchmarks.append((name, step, score, kwargs))


def test_samples_read_trajectories_passed_as_mappings():
    """XTuner's RL trainer saves trajectories as plain dicts nested per prompt.

    Read as objects instead, every field comes back empty and the sample table
    fills with blank rows, so this pins the mapping form and the grouping that
    only the nesting carries.
    """

    writer = _RecordingWriter()
    adapter = XtunerAdapter(writer)
    data_groups = [
        # One prompt, two rollouts: one right, one wrong.
        [
            {
                "messages": [{"role": "user", "content": "2+2?"}],
                "response_str": "4",
                "reward": 1.0,
                "reward_model": {"ground_truth": "4", "data_source": "openai/gsm8k"},
            },
            {
                "messages": [{"role": "user", "content": "2+2?"}],
                "response_str": "5",
                "reward": 0.0,
                "reward_model": {"ground_truth": "4", "data_source": "openai/gsm8k"},
            },
        ],
        # A second prompt, fully solved.
        [
            {
                "messages": [{"role": "user", "content": "3+3?"}],
                "response_str": "6",
                "reward": 1.0,
                "reward_model": {"ground_truth": "6", "data_source": "openai/gsm8k"},
            },
        ],
    ]

    adapter.on_samples(7, data_groups)

    assert len(writer.samples) == 3, "every rollout in every group is written"
    assert writer.samples[0]["reward"] == 1.0
    assert writer.samples[0]["task_name"] == "openai/gsm8k"
    # Prompt grouping survives, so pass rate stays per prompt.
    group_ids = {sample["group_id"] for sample in writer.samples}
    assert len(group_ids) == 2, f"expected one id per prompt, got {group_ids}"
    assert writer.samples[0]["group_id"] == writer.samples[1]["group_id"]
    assert writer.samples[2]["group_id"] not in {writer.samples[0]["group_id"]}

    derived = writer.metrics[0][1]
    # Prompt one passes half its rollouts, prompt two all of them.
    assert derived["sampler.avg_pass"] == 0.75
    assert derived["sampler.pass_zero_ratio"] == 0.0
    assert derived["sampler.pass_one_ratio"] == 0.5
    assert derived["sampler.measurable_prompts"] == 2.0
    # Counts describe rollouts, not groups.
    assert derived["environment.active"] == 3.0
    assert writer.sampler[-1][1]["accepted"] == 3
    assert writer.samples_total == 3


def test_samples_still_accept_a_flat_list_of_objects():
    """The object form older XTuner versions pass must keep working."""

    writer = _RecordingWriter()
    adapter = XtunerAdapter(writer)
    adapter.on_samples(
        3,
        [
            SimpleNamespace(
                rollout_id="r1",
                group_id="g1",
                uid="s1",
                status=SimpleNamespace(value="completed"),
                reward={"score": 1, "pass": True},
                prompt_ids=[1, 2],
                response_ids=[3],
                seq_staleness=0,
                extra_fields={},
            )
        ],
    )

    assert len(writer.samples) == 1
    assert writer.samples[0]["group_id"] == "g1"
    assert writer.metrics[0][1]["sampler.avg_pass"] == 1.0


def _install_against(monkeypatch, trainer_cls):
    """Point the adapter at a stand-in XTuner exposing `trainer_cls`."""

    from probing.ext import xtuner as adapter_module

    deinit()
    module = types.ModuleType("xtuner.v1.train.rl_trainer")
    module.RLTrainer = trainer_cls
    for name in ("xtuner", "xtuner.v1", "xtuner.v1.train"):
        monkeypatch.setitem(sys.modules, name, types.ModuleType(name))
    monkeypatch.setitem(sys.modules, "xtuner.v1.train.rl_trainer", module)
    monkeypatch.setattr(adapter_module, "_INSTALL_ERROR", "", raising=False)
    return adapter_module


def test_install_keeps_the_hooks_a_version_does_have(monkeypatch):
    """XTuner 0.2.0 has `_save_trajectories` but no `_log_step`.

    Requiring `_log_step` made the adapter give up entirely on such a version and
    report nothing, even though trajectories were there to collect.
    """

    class RLTrainer:
        def _save_trajectories(self, data_groups, save_path):
            return None

    adapter_module = _install_against(monkeypatch, RLTrainer)
    try:
        assert install() is True
        state = adapter_module.status()
        assert state["enabled"] is True
        assert state["installed_hooks"] == ["_save_trajectories"]
        assert state["error"] == ""
        # The hook is actually in place, not merely counted.
        assert getattr(RLTrainer._save_trajectories, "_probing_rl_hook", False)
    finally:
        deinit()


def test_trajectory_hook_preserves_prompt_grouping(monkeypatch):
    """The hook must hand the batch over as groups, not flattened.

    Flattening inside the hook drops the prompt grouping before the adapter can
    see it, which silently removes every pass-rate metric even though the samples
    themselves are written fine.
    """

    class RLTrainer:
        def _save_trajectories(self, data_groups, save_path):
            self.saved = (data_groups, save_path)

    _install_against(monkeypatch, RLTrainer)
    try:
        assert install() is True
        writer = _RecordingWriter()
        trainer = RLTrainer()
        trainer._probing_rl_adapter = XtunerAdapter(writer)
        data_groups = [
            [
                {"reward": 1.0, "response_str": "right"},
                {"reward": 0.0, "response_str": "wrong"},
            ],
            [{"reward": 1.0, "response_str": "right"}],
        ]

        trainer._save_trajectories(data_groups, "rollout_idx_9_trajectory.jsonl")

        assert len(writer.samples) == 3, "each rollout reaches the writer"
        assert all(sample["step"] == 9 for sample in writer.samples)
        assert len({sample["group_id"] for sample in writer.samples}) == 2
        derived = writer.metrics[0][1]
        # One prompt at half, one at full: the mean is 0.75.
        assert derived["sampler.avg_pass"] == 0.75
        assert derived["sampler.measurable_prompts"] == 2.0
        # The trainer's own work still happened.
        assert trainer.saved[0] is data_groups
    finally:
        deinit()


def test_install_reports_why_it_could_not_hook_anything(monkeypatch):
    """A trainer with no known hook point must say so rather than go quiet."""

    class RLTrainer:
        def fit(self):
            return None

    adapter_module = _install_against(monkeypatch, RLTrainer)
    try:
        assert install() is False
        state = adapter_module.status()
        assert state["enabled"] is False
        assert state["installed_hooks"] == []
        assert "_log_step" in state["error"], state["error"]
    finally:
        deinit()

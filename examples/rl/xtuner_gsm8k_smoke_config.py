"""Minimal XTuner GRPO + GSM8K smoke config for Probing RL adapter validation.

Env required: WORK_DIR, MODEL_PATH, DATA_PATH, EVAL_DATA_PATH
Optional: XTUNER_RL_NUM_WORKERS (default 4), TOTAL_TRAIN_STEPS (default 2),
TRAIN_BATCH_SIZE (default 8), PROMPT_REPEAT_K (default 2),
MAX_RESPONSE_LENGTH (default 1024), ROLLOUT_TEMPERATURE (default 0.7),
ROLLOUT_TOP_P (default 0.95), ROLLOUT_TOP_K (default 20)
"""

from __future__ import annotations

import os
from pathlib import Path

from xtuner.v1.config import AdamWConfig, FSDPConfig, LRConfig
from xtuner.v1.data_proto.rl_data import SampleParams
from xtuner.v1.datasets.config import DataloaderConfig, DatasetConfig
from xtuner.v1.datasets.rl_tokenize_fn import RLTextTokenizeFnConfig
from xtuner.v1.model import get_model_config_from_hf
from xtuner.v1.rl.advantage import GRPOAdvantageConfig
from xtuner.v1.rl.agent_loop import SingleTurnAgentLoopConfig
from xtuner.v1.rl.agent_loop_manager import (
    AgentLoopManagerConfig,
    SamplerConfig,
    SyncProduceStrategyConfig,
    TaskSpecConfig,
)
from xtuner.v1.rl.evaluator import EvaluatorConfig
from xtuner.v1.rl.judger import GSM8KJudgerConfig
from xtuner.v1.rl.loss import GRPOLossConfig
from xtuner.v1.rl.replay_buffer import SyncReplayBufferConfig
from xtuner.v1.rl.rollout.worker import RolloutConfig
from xtuner.v1.rl.trainer import WorkerConfig
from xtuner.v1.rl.utils import AcceleratorResourcesConfig, CPUResourcesConfig
from xtuner.v1.train.rl_trainer import RLColocateTrainerConfig

work_dir = os.environ["WORK_DIR"]
model_path = os.environ["MODEL_PATH"]
data_path = os.environ["DATA_PATH"]
eval_data_path = os.environ["EVAL_DATA_PATH"]
nnode = int(os.environ.get("WORLD_SIZE", "1"))
num_workers = int(os.environ.get("XTUNER_RL_NUM_WORKERS", "4"))

experimental_name = "probing_xtuner_gsm8k_smoke"
total_train_steps = int(os.environ.get("TOTAL_TRAIN_STEPS", "2"))
# Keep larger than total_train_steps so smoke can finish logging without
# hitting lmdeploy/XTuner weight-IPC incompatibilities on this cluster image.
sync_weights_interval = int(os.environ.get("SYNC_WEIGHTS_INTERVAL", str(max(total_train_steps + 1, 10))))
evaluate_step = sync_weights_interval
train_optimizer_steps = 1
train_batch_size = int(os.environ.get("TRAIN_BATCH_SIZE", "8"))
prompt_repeat_k = int(os.environ.get("PROMPT_REPEAT_K", "2"))
rollout_tp_size = 1
rollout_ep_size = 1
max_prompt_length = 256
max_response_length = int(os.environ.get("MAX_RESPONSE_LENGTH", "1024"))
rollout_temperature = float(os.environ.get("ROLLOUT_TEMPERATURE", "0.7"))
rollout_top_p = float(os.environ.get("ROLLOUT_TOP_P", "0.95"))
rollout_top_k = int(os.environ.get("ROLLOUT_TOP_K", "20"))
pack_max_length = 8 * 1024

resources = AcceleratorResourcesConfig(
    accelerator="GPU",
    num_workers=num_workers * nnode,
    num_cpus_per_worker=8,
    cpu_memory_per_worker=8 * 1024**3,
)

rollout_config = RolloutConfig(
    env=experimental_name,
    device=resources.accelerator,
    model_path=model_path,
    dtype="bfloat16",
    tensor_parallel_size=rollout_tp_size,
    expert_parallel_size=rollout_ep_size,
    gpu_memory_utilization=0.45,
    context_length=max_response_length + max_prompt_length,
    extra_rollout_config=dict(
        lmdeploy_log_level="WARNING",
        lmdeploy_uvicorn_log_level="WARNING",
    ),
)

judger_config = GSM8KJudgerConfig(
    judger_name="openai/gsm8k",
    cpu_resources=CPUResourcesConfig(num_workers=1, num_cpus_per_worker=1),
)

lr_cfg = LRConfig(lr_type="constant", warmup_ratio=0, lr_min=1e-6)
fsdp_cfg = FSDPConfig(torch_compile=False, cpu_offload=False, ep_size=1)
model_cfg = get_model_config_from_hf(Path(model_path))
if hasattr(model_cfg, "balancing_loss_cfg"):
    model_cfg.balancing_loss_cfg = None
if hasattr(model_cfg, "z_loss_cfg"):
    model_cfg.z_loss_cfg = None

optim_cfg = AdamWConfig(lr=1e-6, foreach=False, weight_decay=0.1)
loss_cfg = GRPOLossConfig(
    policy_loss_cfg=dict(
        cliprange_high=0.28,
        cliprange_low=0.2,
        loss_type="vanilla",
        clip_ratio_c=10.0,
        log_prob_diff_min=-20.0,
        log_prob_diff_max=20.0,
    ),
    ignore_idx=-100,
    use_kl_loss=False,
    kl_loss_coef=0.0,
    kl_loss_type="low_var_kl",
    mode="chunk",
    chunk_size=512,
)
train_worker_cfg = WorkerConfig(
    model_cfg=model_cfg,
    load_from=model_path,
    optim_cfg=optim_cfg,
    loss_cfg=loss_cfg,
    lr_cfg=lr_cfg,
    fsdp_cfg=fsdp_cfg,
    sp_size=1,
    optimizer_steps=train_optimizer_steps,
    pack_max_length=pack_max_length,
)

train_dataset = DatasetConfig(name=experimental_name, anno_path=data_path)
tokenizer_config = RLTextTokenizeFnConfig(max_length=max_prompt_length)
train_dataset_cfg = [{"dataset": train_dataset, "tokenize_fn": tokenizer_config}]
dataloader_cfg = DataloaderConfig(
    dataset_config_list=train_dataset_cfg,
    pack_max_length=pack_max_length,
    collator="fake_collator",
    pack_level="none",
)
sampler_config = SamplerConfig(
    dataloader_cfg=dataloader_cfg,
    prompt_repeat_k=prompt_repeat_k,
)
training_sample_params = SampleParams(
    max_tokens=max_response_length,
    top_k=rollout_top_k,
    top_p=rollout_top_p,
    temperature=rollout_temperature,
    min_tokens=0,
)
agent_loop_config = SingleTurnAgentLoopConfig(
    hf_checkpoint=model_path,
    sample_params=training_sample_params,
)
agent_loop_manager_cfg = AgentLoopManagerConfig(
    tasks=TaskSpecConfig(
        task_name="train_task",
        agent_loop_config=agent_loop_config,
        judger_config=judger_config,
        produce_strategy_config=SyncProduceStrategyConfig(),
        sampler_config=sampler_config,
    ),
)

eval_dataset = DatasetConfig(
    name=experimental_name, anno_path=eval_data_path, sample_ratio=1.0
)
eval_dataset_cfg = [{"dataset": eval_dataset, "tokenize_fn": tokenizer_config}]
eval_dataloader_cfg = DataloaderConfig(
    dataset_config_list=eval_dataset_cfg,
    pack_max_length=pack_max_length,
    collator="fake_collator",
    pack_level="none",
)
eval_sampler_config = SamplerConfig(
    dataloader_cfg=eval_dataloader_cfg,
    prompt_repeat_k=1,
)
evaluation_sample_params = SampleParams(
    max_tokens=max_response_length,
    top_k=1,
    top_p=1.0,
    temperature=0.0,
    min_tokens=0,
)
eval_agent_loop_manager_cfg = AgentLoopManagerConfig(
    tasks=TaskSpecConfig(
        task_name="eval_task",
        agent_loop_config=SingleTurnAgentLoopConfig(
            hf_checkpoint=model_path,
            sample_params=evaluation_sample_params,
        ),
        judger_config=judger_config,
        sampler_config=eval_sampler_config,
    ),
)

trainer = RLColocateTrainerConfig(
    resources=resources,
    train_worker_cfg=train_worker_cfg,
    rollout_config=rollout_config,
    tokenizer_path=model_path,
    replay_buffer_config=SyncReplayBufferConfig(),
    agent_loop_manager_cfg=agent_loop_manager_cfg,
    eval_agent_loop_manager_cfg=eval_agent_loop_manager_cfg,
    evaluator_config=EvaluatorConfig(compute_metric_func=None),
    load_from=model_path,
    total_train_steps=total_train_steps,
    train_batch_size=train_batch_size,
    sync_weights_interval=sync_weights_interval,
    advantage_estimator_config=GRPOAdvantageConfig(eps=1e-8),
    enable_evaluate=True,
    enable_initial_evaluate=True,
    evaluate_step=evaluate_step,
    work_dir=work_dir,
    seed=123,
    debug_rollout=False,
    exp_tracker="jsonl",
)

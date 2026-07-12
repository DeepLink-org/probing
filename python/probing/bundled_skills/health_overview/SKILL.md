---
name: health_overview
description: >
  One-shot health check: CPU, GPU, tables, torch step progress, TorchProbe overhead, cluster nodes, NCCL profiler health
category: triage
tables: [cpu.utilization, gpu.utilization, python.torch_trace, python.torch_step_timing, cluster.nodes, nccl.profiler_counters]
tags: [overview, doctor, triage, 健康检查]
keywords:
  en: ['health', 'overview', 'status', 'doctor', 'checkup']
  zh: ['健康', '概览', '怎么样', '正常吗', 'doctor', '体检']
parameters:
  sample_limit: { type: integer, default: 5 }
---

# Training health overview

快速扫描目标进程：有哪些表、CPU/GPU 最近采样、训练是否在推进、TorchProbe shadow 开销、集群节点与 NCCL profiler 计数器。
适合作为 agent 的默认第一步，或用户不确定从哪查起时使用。

## Parameters

- `sample_limit` (integer, default `5`): Recent CPU/GPU samples to show

## Steps (summary)

1. `available_tables` — `information_schema` 可见表
2. `host_cpu` / `gpu_mem` — 最近 CPU/GPU 采样
3. `latest_torch_step` — `python.torch_trace` 最新 `local_step`
4. `torch_probe_overhead` — `python.torch_step_timing` dispatch/hook tax（80-step 窗口）
5. `cluster_nodes` — `cluster.nodes` 存活（若有）
6. `nccl_data_integrity` — `nccl.profiler_counters` 最新一行（若有 NCCL profiler）

## Related skills

- 训练变慢 → skill: module_bottleneck 或 slow_rank
- 显存上涨 → skill: memory_leak
- 进程卡住 → skill: training_hang
- NCCL watchdog → skill: watchdog_timeout

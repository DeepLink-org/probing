---
name: health_overview
description: >
  One-shot health check: CPU, GPU, tables, recent training activity
category: triage
tables: [cpu.utilization, gpu.utilization, python.torch_trace, cluster.nodes]
tags: [overview, doctor, triage, 健康检查]
keywords:
  en: ['health', 'overview', 'status', 'doctor', 'checkup']
  zh: ['健康', '概览', '怎么样', '正常吗', 'doctor', '体检']
parameters:
  sample_limit: { type: integer, default: 5 }
---

# Training health overview

快速扫描目标进程：有哪些表、CPU/GPU 最近采样、训练是否在推进。
适合作为 agent 的默认第一步，或用户不确定从哪查起时使用。

## Parameters

- `sample_limit` (integer, default `5`): Recent CPU/GPU samples to show

## Related skills

- 训练变慢 → skill: module_bottleneck 或 slow_rank
- 显存上涨 → skill: memory_leak
- 进程卡住 → skill: training_hang

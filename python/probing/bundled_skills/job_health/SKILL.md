---
name: job_health
description: >
  Job-level health: cluster alive, step lag, slowdown proxy, comm trend
category: triage
tables: [cluster.nodes, python.comm_collective, global.python.comm_collective, python.trace_event]
tags: [job, slowdown, health, 作业]
keywords:
  en: ['job health', 'job slow', 'slowdown', 'training degraded', 'step lag']
  zh: ['作业健康', 'job 慢', '整体变慢', 'slowdown', 'step 落后']
parameters:
  step_window: { type: integer, default: 200 }
  step_lag_threshold: { type: integer, default: 5 }
  use_global: { type: boolean, default: true }
---

# Job health & slowdown

从 **整 job** 视角看训练是否健康，而不是单个 rank 的单次 collective。

## 看什么

| 步骤 | 含义 |
|------|------|
| cluster_snapshot | 多少节点 alive / dead |
| rank_step_lag | 哪些 rank 的 `global_step` 明显落后（hang 信号） |
| job_slowdown_proxy | 各 step 上最慢 rank vs 最快 rank 的比值均值（§ federation 3.2） |
| comm_trend_by_step | 最近 step 通信总耗时是否爬升 |
| train_step_pacing | `train.step` span 的 wall time（若已开启 phase hook） |

## Parameters

- `step_window` (default `200`): 分析最近多少 global_step
- `step_lag_threshold` (default `5`): 落后 cluster max step 多少步算异常
- `use_global` (default `true`): rank0 上 fan-out 到全集群

## 用法

```bash
probing -t rank0:8080 skill run job_health --global
probing -t rank0:8080 skill run job_health --set step_window=500
```

## Related skills

- 长期最慢 rank → `persistent_straggler`
- 当前 step 谁慢 → `slow_rank`
- 进程卡住 → `training_hang`

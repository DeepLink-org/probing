---
name: slow_rank
description: >
  Find straggler ranks via collective latency imbalance
category: distributed
tables: [python.comm_collective, global.python.comm_collective, nccl.proxy_ops]
tags: [rank, straggler, distributed, collective, 拖后腿]
keywords:
  en: ['slow rank', 'straggler', 'lagging rank', 'which rank']
  zh: ['慢 rank', '拖后腿', 'straggler', '掉队', '哪个 rank', 'rank 慢']
parameters:
  step_window: { type: integer, default: 20 }
  use_global: { type: boolean, default: True }
---

# Find slow rank (straggler)

对比各 rank 的 collective 延迟，找出明显偏慢的 straggler。
单机多卡时 rank 列来自 torch.distributed；多机时使用 global.* 并带 _host/_rank 标签。

## Parameters

- `step_window` (integer, default `20`): Include collectives from the last N global_steps
- `use_global` (boolean, default `True`): Query global.python.comm_collective for cross-node fan-out

## Related skills

- 某 rank 持续最慢 → 检查该节点 GPU/网络/数据: skill: gpu_pressure
- 栈卡在 collective → skill: training_hang
- 模块级热点 → skill: module_bottleneck (在慢 rank 上 inject)
- 有 NCCL profiler → skill: nccl_culprit_victim

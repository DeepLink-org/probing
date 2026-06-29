---
name: persistent_straggler
description: >
  Find ranks that are slow on many steps (worst_fraction), not just once
category: distributed
tables: [python.comm_collective, global.python.comm_collective]
tags: [straggler, persistent, worst_fraction]
keywords:
  en: ['persistent straggler', 'worst fraction', 'always slow', 'chronic straggler']
  zh: ['长期慢', '一直慢', 'worst_fraction', '总是最慢']
parameters:
  step_window: { type: integer, default: 300 }
  worst_fraction_threshold: { type: number, default: 0.25 }
  use_global: { type: boolean, default: true }
---

# Persistent straggler (worst_fraction)

`slow_rank` 看 **当前窗口里谁平均最慢**；本 skill 看 **谁在多少 step 上拿「最慢 rank」**。

## worst_fraction

对每个 rank 统计：在窗口内有多少 step 上，该 rank 的 collective 总耗时等于该 step 的最大值。

- `worst_fraction ≈ 1.0` — 几乎每步都是最慢（ chronic straggler）
- `worst_fraction ≈ 0.05` — 偶发

## Parameters

- `step_window` (default `300`)
- `worst_fraction_threshold` (default `0.25`) — `persistent_offenders` 步骤的过滤阈值
- `use_global` (default `true`)

## 用法

```bash
probing -t rank0:8080 skill run persistent_straggler --global
probing -t rank0:8080 skill run persistent_straggler --set worst_fraction_threshold=0.4 --set step_window=500
```

## Related skills

- Job 整体变慢 → `job_health`
- 当前 step lag → `slow_rank`
- NCCL culprit/victim → `nccl_culprit_victim`

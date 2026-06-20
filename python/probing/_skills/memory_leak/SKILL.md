---
name: memory_leak
description: >
  Detect monotonic GPU memory growth across training steps
category: memory
tables: [python.torch_trace, gpu.utilization]
tags: [memory, leak, OOM, 显存, 泄漏]
keywords:
  en: ['memory leak', 'OOM', 'memory growing', 'out of memory']
  zh: ['泄漏', 'OOM', '显存涨', '内存涨', 'out of memory', '阶梯']
parameters:
  min_steps: { type: integer, default: 10 }
  step_skip: { type: integer, default: 2 }
---

# GPU memory leak detection

检测 python.torch_trace 中 allocated 是否随 step 单调上涨，
并结合 gpu.utilization 看设备级显存趋势。

## Parameters

- `min_steps` (integer, default `10`): Minimum steps required for trend analysis
- `step_skip` (integer, default `2`): Skip first N steps (discovery / warmup)

## Related skills

- 某模块 delta 大 → 检查是否 cache 了 tensor / 未 detach
- 仅 torch_trace 涨而 gpu.utilization 平稳 → 可能是统计口径问题
- Linux OOM → SELECT * FROM process.kmsg WHERE message LIKE '%oom%'

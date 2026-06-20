---
name: training_hang
description: >
  Training stalled — capture stacks and thread/blocking hints
category: reliability
tables: [python.backtrace, cpu.utilization, python.trace_event, python.comm_collective]
tags: [hang, stall, deadlock, 卡住, 不动]
keywords:
  en: ['hang', 'stuck', 'stall', 'frozen', 'deadlock', 'not progressing']
  zh: ['卡住', 'hang', '不动', '停住', 'deadlock', '死锁', 'loss 不更新']
parameters:
  stack_depth: { type: integer, default: 20 }
---

# Training hang diagnosis

Loss 不更新、进程仍存活但无进展时使用。
组合：实时混合栈 + CPU 线程 wchan + 最近 trace 活动。

## Parameters

- `stack_depth` (integer, default `20`): Backtrace frames to show

## Related skills

- 栈在 dist/barrier → 检查各 rank 是否都存活: probing list / cluster nodes
- 栈在 DataLoader → 减少 num_workers 或检查数据路径
- 需要持续采样 → SET probing.pprof.sample_freq=99 后 skill: module_bottleneck

---
name: module_bottleneck
description: >
  Find slowest PyTorch modules in recent steps
category: performance
tables: [python.torch_trace]
tags: [slow, bottleneck, module, torch, 慢, 瓶颈]
keywords:
  en: ['slow', 'bottleneck', 'hotspot', 'which module', 'slowdown']
  zh: ['慢', '瓶颈', '哪个模块', '变慢', 'hotspot', '热点']
parameters:
  recent_steps: { type: integer, default: 10 }
  stage_filter: { type: string, default: post forward }
---

# PyTorch module bottleneck

基于 python.torch_trace 的 post-hook duration，找出最近 step 中最耗时的模块。
与 torch.profiler 互补：这是长期采样、模块级、低开销视图。

## Parameters

- `recent_steps` (integer, default `10`): Analyze the last N training steps
- `stage_filter` (string, default `post forward`): Hook stage to aggregate (post forward | post step)

## Related skills

- 打开 Torch 火焰图 (profiling/torch) 看调用栈分布
- 分布式变慢 → skill: slow_rank
- 需要 CPU 栈 → SET probing.pprof.sample_freq=99, profiling/pprof

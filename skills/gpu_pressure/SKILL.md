---
name: gpu_pressure
description: >
  GPU memory and utilization headroom
category: memory
tables: [gpu.utilization, gpu.devices, python.torch_trace]
tags: [gpu, VRAM, utilization, 显存, 利用率]
keywords:
  en: ['GPU memory', 'VRAM', 'GPU utilization', 'GPU idle']
  zh: ['显存不够', 'GPU 利用率', 'VRAM', '显存占用', 'GPU 空闲']
parameters:
  sample_limit: { type: integer, default: 20 }
---

# GPU memory and utilization pressure

查看 gpu.utilization 采样与 python.torch_trace 中的 allocated 是否一致，
判断是「真 OOM 风险」还是「利用率低 / 内存碎片」。

## Parameters

- `sample_limit` (integer, default `20`):

## Related skills

- 显存持续上涨 → skill: memory_leak
- MPS Mac → torch.mps.current_allocated_memory 已在 torch_trace 中
- 启用更细 profiling → probing.torch.profiling=on,random:0.1

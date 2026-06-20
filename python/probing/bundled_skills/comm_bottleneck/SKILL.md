---
name: comm_bottleneck
description: >
  Rank collective ops by total and p99 latency
category: distributed
tables: [python.comm_collective, nccl.proxy_ops, rdma.mlx_hca]
tags: [NCCL, collective, communication, 通信, all_reduce]
keywords:
  en: ['communication slow', 'NCCL', 'collective', 'comm bottleneck']
  zh: ['通信慢', 'NCCL', 'all_reduce', '通信瓶颈', '带宽']
parameters:
  step_window: { type: integer, default: 50 }
  use_global: { type: boolean, default: True }
---

# Communication bottleneck

按 collective op 聚合延迟与传输量，定位通信热点。
适用于「计算不慢但 step 时间长」或「通信占比高」的场景。

## Parameters

- `step_window` (integer, default `50`):
- `use_global` (boolean, default `True`):

## Related skills

- all_reduce 慢 → 检查 bucket 大小、overlap、FP16 compress
- 有 `nccl.proxy_ops` → skill: `nccl_culprit_victim`（culprit/victim 归因）
- RoCE 拥塞 → 查 rdma.mlx_hca 与交换机 ECN 配置

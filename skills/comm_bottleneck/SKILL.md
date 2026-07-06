---
name: comm_bottleneck
description: >
  Rank collective ops by total and p99 latency
category: distributed
tables: [nccl.coll_perf, nccl.proxy_ops, python.comm_collective, rdma.mlx_hca]
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

## 数据层次（勿混淆计时语义）

| 层 | 表 | 计时 |
|----|----|------|
| NCCL profiler 插件（**优先**） | `nccl.coll_perf`、`nccl.proxy_ops` | NCCL 原生事件，执行时间/带宽精准（见 `timing_source`） |
| Torch API 层插桩（回退） | `python.comm_collective` | Python 墙钟，launch 层粗粒度；独有 `global_step` 上下文 |

NCCL 插件启用时 Torch 侧插桩默认关闭（`probing.torch.collective.enable=1` 可强制同开）。

## Parameters

- `step_window` (integer, default `50`):
- `use_global` (boolean, default `True`):

## Related skills

- all_reduce 慢 → 检查 bucket 大小、overlap、FP16 compress
- 有 `nccl.proxy_ops` → skill: `nccl_culprit_victim`（culprit/victim 归因）
- RoCE 拥塞 → 查 rdma.mlx_hca 与交换机 ECN 配置

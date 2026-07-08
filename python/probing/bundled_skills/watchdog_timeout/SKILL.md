---
name: watchdog_timeout
description: >
  Diagnose PyTorch NCCL watchdog timeouts with Flight Recorder collective
  sequence alignment.
category: distributed
tables: [python.torch_nccl_flight_record, global.python.torch_nccl_flight_record, python.torch_nccl_pg_status]
tags: [NCCL, watchdog, timeout, Flight Recorder, desync, collective]
keywords:
  en: ['NCCL watchdog', 'timeout', 'flight recorder', 'collective mismatch', 'desync']
  zh: ['NCCL watchdog', '超时', 'Flight Recorder', 'collective 不一致', '反同步', '卡住']
parameters:
  seq_window: { type: integer, default: 200 }
  use_global: { type: boolean, default: true }
---

# Watchdog timeout diagnosis

用于 PyTorch NCCL watchdog timeout、collective deadlock、rank 间 collective 顺序不一致等场景。

这个 skill 会先触发 PyTorch Flight Recorder snapshot，并写入：

- `python.torch_nccl_flight_record`
- `python.torch_nccl_pg_status`

然后按 `pg_id + collective_seq_id` 对齐各 rank，检查 missing rank、collective type mismatch、shape/dtype mismatch、state mismatch。

## Prerequisites

- PyTorch >= 2.5
- 目标进程内存在 `torch._C._distributed_c10d._dump_nccl_trace`
- 最好提前设置 `TORCH_NCCL_TRACE_BUFFER_SIZE=2000`
- timeout 自动落盘可额外设置 `TORCH_NCCL_DUMP_ON_TIMEOUT=1`

## Reading results

- `ranks_seen < expected_world_size`：某些 rank 没有进入同一个 collective seq，常见于 CPU-side hang 或分支发散。
- `op_min != op_max`：同一个 seq 上不同 rank 发起了不同 collective。
- `input_sizes_min != input_sizes_max` 或 dtype 不同：collective 参数不一致。
- `state_min != state_max`：某些 rank 未完成、未开始或处于不同状态。

## Related skills

- 已进入 NCCL 但想看 culprit/victim → `nccl_culprit_victim`
- 训练仍存活但无进展 → `training_hang`
- 粗粒度通信慢 → `comm_bottleneck`

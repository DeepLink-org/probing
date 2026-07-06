---
name: sre_triage
description: >
  SRE first-response triage for distributed training incidents. Automates the
  manual checks from PyTorch/NCCL debugging runbooks.
category: sre
tables:
  [cluster.nodes, cpu.utilization, gpu.utilization, python.comm_collective,
   python.torch_nccl_flight_record, nccl.proxy_ops, rdma.mlx_hca, process.kmsg]
tags: [SRE, runbook, oncall, triage, NCCL, hang, OOM, network]
keywords:
  en: ['SRE', 'oncall', 'runbook', 'incident', 'triage', 'NCCL timeout', 'hang']
  zh: ['SRE', '值班', '应急', 'runbook', '分诊', '事故', '卡住', 'NCCL 超时']
parameters:
  use_global: { type: boolean, default: true }
  seq_window: { type: integer, default: 200 }
  sample_limit: { type: integer, default: 20 }
---

# SRE triage

面向值班第一响应：把原本需要人工 ssh、grep、py-spy、看 NCCL/IB/GPU/内核日志的流程，尽量转成 probing 可执行证据链。

这个 skill 不追求一次性给出最终根因，而是自动回答：

1. 集群/rank 是否都在线？
2. 训练是否还在推进？
3. 是否有 NCCL watchdog / collective desync 证据？
4. 是否像 CPU hang、GPU/NCCL wait、网络拥塞、OOM/Xid？
5. 下一步应该跑哪个更专门的 skill？

## Reading results

- `incident_snapshot`：当前有哪些表，决定能自动化到什么深度。
- `cluster_nodes`：rank/host 是否齐，是否有 dead/stale 节点。
- `blocked_threads` / `kernel_wait_points`：类似人工 py-spy/strace 的第一层替代。
- `flight_recorder_alignment`：自动化 Flight Recorder / fr_trace 的核心对齐判断。
- `nccl_wait_summary`：进入 NCCL 后的 culprit/victim 信号。
- `rdma_network_hints` / `kernel_fault_hints`：网络或硬件灰故障线索。

## Related skills

- NCCL timeout 细查 → `watchdog_timeout`
- 已进入 NCCL wait → `nccl_culprit_victim`
- 训练卡住但不是 NCCL → `training_hang`
- 显存/OOM → `memory_leak` / `gpu_pressure`

---
name: nccl_culprit_victim
description: >
  NCCL proxy wait decomposition — culprit (send_gpu_wait) vs victim (recv_wait)
category: distributed
tables: [nccl.proxy_ops, nccl.net_qp, global.nccl.proxy_ops]
tags: [NCCL, culprit, victim, straggler, wait, proxy]
keywords:
  en: ['NCCL culprit', 'NCCL victim', 'recv wait', 'gpu wait', 'slow rank NCCL', 'proxy wait']
  zh: ['NCCL 慢 rank', '罪魁祸首', '受害者', 'recv_wait', 'gpu_wait', '等待分解']
parameters:
  seq_window: { type: integer, default: 20 }
  use_global: { type: boolean, default: true }
---

# NCCL culprit / victim attribution

Uses **NCCL profiler plugin** wait decomposition (`nccl.proxy_ops`):

- **Culprit** (slow local GPU): high `send_gpu_wait_ns`
- **Victim** (waiting on peers / network): high `recv_wait_ns`
- **Receiver congestion** (v4 ABI only): high `send_peer_wait_ns` — this rank's
  sends are stalled waiting for the *peer's* clear-to-send credits; investigate
  that peer

Requires `NCCL_PROFILER_PLUGIN` + `probing-nccl-profiler` (or mock data via `PROBING_NCCL_MOCK` on macOS).

## Parameters

- `seq_window` (integer, default `20`): recent collective sequences to analyze
- `use_global` (boolean, default `true`): fan-out with `global.nccl.proxy_ops`

## Related skills

- Coarse collective latency → `comm_bottleneck` / `slow_rank` (`python.comm_collective`)
- IB QP timing → `nccl.net_qp` when NetPlugin mask enabled

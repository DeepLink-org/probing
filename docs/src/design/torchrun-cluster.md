# Torchrun hierarchical cluster heartbeat

Multi-process `torchrun` jobs **auto-register** cluster nodes when probing is injected, powering `probing cluster nodes`, the Web cluster page, and `global.*` federation queries. Implementation lives in L3 `probing/server` (Rust). It does **not** block `init_process_group` and does **not** write torch rendezvous keys.

## When it starts

| Condition | Notes |
|-----------|--------|
| `PROBING=1/2` | Probing injected |
| `WORLD_SIZE > 1` | Single-process jobs skip cluster |
| `PROBING_TORCHRUN_CLUSTER≠0` | Default **on** |
| `PROBING_CLUSTER_REPORT≠0` | Default **on** |
| Not elastic supervisor | torchrun parent process skips HTTP bind |

The Rust ctor (`import probing`) calls `maybe_start_torchrun_cluster()`: bind HTTP, publish master/local0 on TCPStore, start the Tokio heartbeat worker.

## Hierarchy

```text
leaf (local_rank>0)  ──PUT──►  local0 (local_rank=0 on same node)
local0 (not global0) ──PUT──►  master (global rank 0)
global rank 0        ──PUT──►  local master view
```

Discovery keys: `probing/torchrun/<run_id>/master` and `.../node/<group_rank>/local0` on the job TCPStore (same endpoint as rendezvous, separate key namespace).

## Environment variables

See [Environment variables](../reference/env-vars.md) for the full list. Highlights:

| Variable | Default | Purpose |
|----------|---------|---------|
| `PROBING_TORCHRUN_CLUSTER` | `1` | Enable torchrun cluster |
| `PROBING_CLUSTER_REPORT` | `1` | Periodic heartbeat |
| `PROBING_CLUSTER_REPORT_INTERVAL_SEC` | `10` | Base interval (seconds) |
| `PROBING_CLUSTER_STALE_SEC` | `25` | Mark node `dead` after silence |
| `PROBING_CLUSTER_REPORT_MAX_INTERVAL_SEC` | `120` | Backoff cap (clamped below stale) |

**Stale vs backoff:** effective max interval = `min(configured_max, STALE_SEC - STALE_SEC/4 - 1)`. With default stale=25, max ≈ **18s**. For ~60s stable heartbeats, raise `PROBING_CLUSTER_STALE_SEC` (≥90 recommended).

## Presets (`PROBING_CLUSTER_PRESET`)

`examples/run_cluster_multinode.sh` supports:

| Preset | Use case |
|--------|----------|
| `demo` (default) | Local multinode demo |
| `fast` | Faster convergence visibility (5s interval) |
| `steady` | Long runs, lower CPU (90s stale) |

```bash
PROBING_CLUSTER_PRESET=fast ./examples/run_cluster_multinode.sh 2 2
```

## Demo

```bash
./examples/run_cluster_multinode.sh
probing -t rank0-host:18080 cluster nodes
```

Cluster heartbeat starts from the **Rust ctor only**; `init_process_group` is not patched. Python `probing.torchrun_cluster.setup_torchrun_cluster()` remains for explicit calls and tests.

See also [Distributed architecture](distributed.md).

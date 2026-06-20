# NCCL profiler plugin

Fine-grained **NCCL wait decomposition** for distributed training: distinguish a **culprit** rank (local GPU slow to produce data) from a **victim** rank (waiting on peers or the network).

This is **Path 3** in [Extensibility](extensibility.md)—a Rust `cdylib` loaded by NCCL, not a Python table plugin.

## When to use

| Signal | Tool |
|--------|------|
| Step time high, unsure if comm or compute | `python.comm_collective` + skill `comm_bottleneck` |
| Which rank is the straggler? | skill `slow_rank` |
| Straggler identified — **why** (GPU vs network wait)? | `nccl.proxy_ops` + skill `nccl_culprit_victim` |
| Suspect RoCE / IB congestion | `nccl.net_qp` + `rdma.mlx_hca` |

Coarse collective tracing (`python.comm_collective`) works with `PROBING=1` only. The NCCL profiler plugin requires **NCCL ≥ 2.26** (PyTorch **2.8+** recommended).

## Quick start (Linux training)

```bash
pip install probing   # wheel bundles libprobing_nccl_profiler.so on Linux

export NCCL_PROFILER_PLUGIN=$(python -m probing.nccl --plugin-path)
export NCCL_PROFILE_EVENT_MASK=$(python -m probing.nccl --event-mask)   # default 26
export PROBING=2

torchrun --nproc_per_node=8 train.py

# Same process or after inject:
probing -t <pid> skill run nccl_culprit_victim
probing -t <pid> query "
  SELECT rank, sum(send_gpu_wait_ns) AS gpu_wait, sum(recv_wait_ns) AS recv_wait
  FROM nccl.proxy_ops
  GROUP BY rank
  ORDER BY recv_wait DESC"
```

### Optional: NetPlugin (IB QP timing)

```bash
export NCCL_PROFILE_EVENT_MASK=154   # 26 + NetPlugin bit 128
probing -t <pid> query "SELECT * FROM nccl.net_qp LIMIT 20"
```

## macOS / dev without NCCL

```bash
PROBING=1 PROBING_NCCL_MOCK=1 python -m probing.nccl --seed-mock
probing -t <pid> skill run nccl_culprit_victim
```

On macOS, `PROBING_NCCL_MOCK=auto` (default) seeds mock tables when `PROBING=1` and no plugin `.so` is present.

Mock scenario:

- **rank 2** — culprit (`send_gpu_wait_ns` high)
- **rank 5** — victim (`recv_wait_ns` high)

## Tables

### `nccl.proxy_ops`

Per NCCL proxy operation, with ProxyStep waits aggregated at op stop.

| Column | Meaning |
|--------|---------|
| `ts` | Event timestamp (ns) |
| `rank` | `torch.distributed` rank |
| `tp_rank`, `pp_rank`, `dp_rank` | Parallel roles from env (`TP_RANK`, `PP_RANK`, `DP_RANK`, Megatron names); `-1` if unset |
| `comm_hash` | NCCL communicator hash |
| `coll_func` | Collective name (`AllReduce`, …) |
| `seq` | Collective sequence number |
| `channel_id` | NCCL channel |
| `peer` | Peer rank for this proxy op |
| `is_send` | `1` = send proxy, `0` = recv |
| `n_steps` | ProxyStep count aggregated |
| `trans_bytes` | Bytes transferred |
| `send_gpu_wait_ns` | **Culprit signal** — local GPU not ready to send |
| `send_wait_ns` | Send-side network wait |
| `recv_wait_ns` | **Victim signal** — waiting on peer data |
| `recv_flush_wait_ns` | Recv flush wait |

Multi-node: `global.nccl.proxy_ops` with `_host`, `_addr`, `_rank` federation columns.

### `nccl.net_qp`

IB queue-pair completion timing (NetPlugin mask). Columns: `ts`, `rank`, `device`, `qp_num`, `wr_id`, `opcode`, `length`, `duration_ns`.

## Culprit vs victim

From NCCL ProxyStep state transitions (paper mapping):

- **Culprit** — dominant `send_gpu_wait_ns` on a rank: that GPU is slow to produce tensors for the collective.
- **Victim** — dominant `recv_wait_ns`: the rank spends time waiting for peers or the network.

A single rank can appear as culprit for one collective and victim for another. Compare both columns per rank; use `tp_rank`/`pp_rank`/`dp_rank` to align with Megatron-style topology.

## Diagnostic skill: `nccl_culprit_victim`

Bundled under `skills/nccl_culprit_victim/` (wheel: `python/probing/_skills/`).

```bash
probing skill list
probing -t <pid> skill run nccl_culprit_victim
probing -t <pid> skill run nccl_culprit_victim --set seq_window=50 --global
```

Steps include:

1. Per-rank wait summary (`send_gpu_wait_ns` / `recv_wait_ns`)
2. Culprit ranking (by `send_gpu_wait_ns`)
3. Victim ranking (by `recv_wait_ns`)
4. Role-aligned view (`tp` / `pp` / `dp`)
5. Optional `global.nccl.proxy_ops` fan-out
6. Optional `nccl.net_qp` hint

Related skills: `slow_rank`, `comm_bottleneck` (coarse layer; optionally join `nccl.proxy_ops` when present).

## Environment variables

| Variable | Purpose |
|----------|---------|
| `NCCL_PROFILER_PLUGIN` | Path to `libprobing_nccl_profiler.so` |
| `NCCL_PROFILE_EVENT_MASK` | Event mask; default `26` = Coll \| ProxyOp \| ProxyStep |
| `PROBING_DATA_DIR` | Memtable directory (default `/dev/shm/probing`) |
| `PROBING_NCCL_MOCK` | `auto` / `1` / `0` — mock tables for dev |
| `TP_RANK`, `PP_RANK`, `DP_RANK` | Written into `nccl.proxy_ops` role columns |

CLI helpers:

```bash
python -m probing.nccl --plugin-path
python -m probing.nccl --event-mask
python -m probing.nccl --seed-mock --ranks 8 --ops 5
```

## Build from source

```bash
make nccl-profiler-lib    # Linux .so → python/probing/libs/
cargo test -p probing-nccl-profiler
```

Crate: `probing/extensions/nccl-profiler/`. See crate [README](../../../probing/extensions/nccl-profiler/README.md) for architecture (slot pools, Coll→ProxyOp→ProxyStep hierarchy, batch flush).

## Smoke test checklist (P0)

1. `python -c "import torch; print(torch.__version__, torch.cuda.nccl.version())"` — NCCL ≥ 2.26
2. `NCCL_PROFILER_PLUGIN` set before `torchrun`
3. After a few collectives: `SELECT count(*) FROM nccl.proxy_ops` > 0
4. `probing skill run nccl_culprit_victim` returns rank breakdown

## See also

- [Distributed training](distributed.md) — cluster fan-out, `global.*`
- [Extensibility](extensibility.md) — Path 1 (table plugin), Path 2 (skills), Path 3 (this plugin)
- [AGENTS.md](../../../AGENTS.md) — agent skill install and routing

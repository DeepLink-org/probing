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

Coarse collective tracing (`python.comm_collective`) works with `PROBING=1` only. The NCCL profiler plugin requires **NCCL ≥ 2.26** (PyTorch **2.8+** recommended); it exports both **`ncclProfiler_v4`** (NCCL ≥ 2.27, preferred) and **`ncclProfiler_v3`** (NCCL 2.26) — NCCL negotiates the highest version automatically.

## Three collective data sources — keep them apart

probing has three independent collective-communication collectors. They have
**different timing semantics** and must not be conflated:

| Source | Tables | What it measures | Role |
|--------|--------|------------------|------|
| **NCCL profiler plugin** (this doc) | `nccl.coll_perf`, `nccl.proxy_ops`, `nccl.inflight_ops`, `nccl.net_qp` | NCCL-native events: reconstructed execution time, wait decomposition, bandwidth | **Precise source of truth** |
| Torch-API tracer (legacy, `probing/profiling/collective/`) | `python.comm_collective` | Python wall-clock around the `torch.distributed` API call (launch layer) | Coarse fallback; carries `global_step` context |
| PyTorch Flight Recorder bridge | `python.torch_nccl_flight_record`, `python.torch_nccl_pg_status` | torch's internal watchdog ring buffer | Watchdog-timeout / desync forensics |

Rules of engagement:

- When the plugin is active (`NCCL_PROFILER_PLUGIN` set), the Torch-API tracer
  is **disabled by default** — recording the same collectives twice with
  conflicting timing only creates confusion. Force both with
  `SET probing.torch.collective.enable=1` (e.g. when you need per-step
  `global_step` alignment alongside precise timing).
- For execution time, bandwidth, and wait attribution, always query `nccl.*`.
  `python.comm_collective.duration_ms` is **not** NCCL execution time — for
  `async_op` calls it closes at `work.wait()`, otherwise at API return.
- Joining the layers: `nccl.*` rows carry no training step; correlate by
  epoch-ns time window against `python.comm_collective.global_step` if needed.

## Quick start (Linux training)

```bash
pip install probing   # wheel bundles libprobing_nccl_profiler.so on Linux

export NCCL_PROFILER_PLUGIN=$(python -m probing.nccl --plugin-path)
export NCCL_PROFILE_EVENT_MASK=$(python -m probing.nccl --event-mask)   # default 94
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
export NCCL_PROFILE_EVENT_MASK=222   # 94 + NetPlugin bit 128
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
| `trans_bytes` | Bytes transferred (v4: summed from per-step `transSize` updates) |
| `send_gpu_wait_ns` | **Culprit signal** — local GPU not ready to send |
| `send_peer_wait_ns` | Waiting for receiver clear-to-send credits (**v4 ABI only**, 0 on v3) — receiver-congestion signal |
| `send_wait_ns` | Send-side network wait |
| `recv_wait_ns` | **Victim signal** — waiting on peer data |
| `recv_flush_wait_ns` | Recv flush wait |

Multi-node: `global.nccl.proxy_ops` with `_host`, `_addr`, `_rank` federation columns.

> `ts` columns in all `nccl.*` tables are **UNIX-epoch nanoseconds**, so
> timestamps are comparable across ranks/hosts in `global.nccl.*` queries.

### `nccl.coll_perf`

Per collective / P2P operation.

**Timing model.** NCCL's own docs state that a collective's `stopEvent` only
marks the end of the **host-side enqueue** — the kernel and proxy threads keep
working after it. Following the official ext-profiler recommendation, the
plugin reference-counts child events (`ProxyOp`, `KernelCh`) and reconstructs
the real execution window from them. The `timing_source` column records which
signal was available:

| `timing_source` | Window | Quality |
|-----------------|--------|---------|
| `kernel_gpu` | GPU **globaltimer** window: `kernelCh.pTimer` (start) + `KernelChStop` state (stop) | Best — device clock, **v4 ABI only** |
| `kernel_ch` | Kernel-channel activity observed by the proxy thread (`ncclProfileKernelCh`) | NCCL's own kernel-activity signal, host clock |
| `proxy` | Proxy-op start→stop envelope | Good for inter-node ops |
| `enqueue` | Coll start→stop (launch only) | Fallback — intra-node ops without proxy/kernel events |

| Column | Meaning |
|--------|---------|
| `ts` | Op completion timestamp (epoch ns) |
| `rank`, `tp_rank`, `pp_rank`, `dp_rank` | Same as `nccl.proxy_ops` |
| `comm_hash`, `coll_func`, `seq` | Collective identity (`seq` = 0 for P2P) |
| `n_ranks` | Communicator size (v4 per-comm `init` metadata; `-1` on v3) |
| `is_p2p` | `1` = Send/Recv, `0` = collective |
| `peer` | P2P peer rank (`-1` for collectives) |
| `count`, `msg_size_bytes`, `dtype` | Payload: element count × dtype size |
| `algo`, `proto`, `n_channels` | NCCL algorithm (Ring/Tree…), protocol (LL/LL128/Simple), channels (v4: P2P too) |
| `exec_time_ns` | Reconstructed execution duration (see `timing_source`) |
| `enqueue_time_ns` | Host-side enqueue duration (NCCL coll start→stop) |
| `timing_source` | `kernel_gpu` / `kernel_ch` / `proxy` / `enqueue` |
| `algobw_gbps` | Algorithm bandwidth `msg_size / exec_time` (GB/s). **Bus bandwidth**: multiply by the collective factor using `n_ranks`, e.g. AllReduce `2(n_ranks-1)/n_ranks`, in SQL |

```sql
-- Slowest AllReduce buckets by bandwidth
SELECT coll_func, msg_size_bytes, AVG(algobw_gbps) AS gbps, COUNT(*) AS n
FROM nccl.coll_perf
GROUP BY coll_func, msg_size_bytes
ORDER BY gbps ASC LIMIT 10
```

### `nccl.inflight_ops`

Periodic watchdog snapshot of operations that **started but never stopped** —
the hang signal that `nccl.proxy_ops` cannot capture (a hung op never reaches
`stop_event`). Columns: `ts`, `rank`, `comm_hash`, `coll_func`, `seq`, `kind`
(`coll`/`p2p`/`proxy_op`), `channel_id`, `peer`, `is_send`, `start_ns`, `age_ns`.

```sql
-- Which rank is stuck, and in what?
SELECT rank, coll_func, seq, kind, MAX(age_ns)/1e9 AS stuck_secs
FROM nccl.inflight_ops
GROUP BY rank, coll_func, seq, kind
ORDER BY stuck_secs DESC
```

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
| `NCCL_PROFILE_EVENT_MASK` | Event mask; default `94` = Coll \| P2P \| ProxyOp \| ProxyStep \| KernelCh |
| `PROBING_DATA_DIR` | Memtable directory (default `/dev/shm/probing`) |
| `PROBING_NCCL_MIN_MSG_BYTES` | Skip ops smaller than this (bytes); default `0` = record all. Same idea as NCCL Inspector's `DUMP_MIN_SIZE_BYTES` |
| `PROBING_NCCL_INFLIGHT_THRESHOLD_SECS` | Watchdog: snapshot in-flight ops older than this into `nccl.inflight_ops`; default `10`, `0` disables |
| `PROBING_NCCL_POOL_SHARDS` | Shard slot pools by comm hash (default `8`, range 1–64); reduces callback lock contention on multi-comm jobs |
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

Crate: `probing/extensions/nccl-profiler/`. See crate [README](https://github.com/DeepLink-org/probing/blob/main/probing/extensions/nccl-profiler/README.md) for architecture (slot pools, Coll→ProxyOp→ProxyStep hierarchy, batch flush).

## Smoke test checklist (P0)

1. `python -c "import torch; print(torch.__version__, torch.cuda.nccl.version())"` — NCCL ≥ 2.26
2. `NCCL_PROFILER_PLUGIN` set before `torchrun`
3. After a few collectives: `SELECT count(*) FROM nccl.proxy_ops` > 0
4. `probing skill run nccl_culprit_victim` returns rank breakdown

## See also

- [Distributed training](distributed.md) — cluster fan-out, `global.*`
- [Extensibility](extensibility.md) — Path 1 (table plugin), Path 2 (skills), Path 3 (this plugin)
- [AGENTS.md](https://github.com/DeepLink-org/probing/blob/main/AGENTS.md) — agent skill install and routing

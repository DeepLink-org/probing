# probing-nccl-profiler

NCCL **profiler plugin** (API v3, NCCL ≥ 2.26) that writes mmap memtables consumable by probing SQL.

## Build

```bash
cargo build -p probing-nccl-profiler --release
# artifact: target/release/libprobing_nccl_profiler.so
```

Requires **Linux** (same as NCCL distributed training).

## macOS / dev without NCCL

Mock data for SQL and skills debugging (no GPU / NCCL required):

```bash
# Manual seed (needs PROBING=1 or probing engine in-process)
PROBING=1 python -m probing.nccl --seed-mock

# macOS: auto-seed on PROBING=1 (default PROBING_NCCL_MOCK=auto)
PROBING=1 python your_script.py
probing -t <pid> query "SELECT rank, recv_wait_ns, send_gpu_wait_ns FROM nccl.proxy_ops"
probing -t <pid> skill run nccl_culprit_victim
```

| Variable | Default (macOS) | Meaning |
|----------|-----------------|--------|
| `PROBING_NCCL_MOCK` | `auto` | `auto` / `1` seeds mock tables; `0` disables |

Mock scenario: rank **2** = culprit (`send_gpu_wait_ns` high), rank **5** = victim (`recv_wait_ns` high).

## Phase 0 — smoke test

```bash
# 1. Confirm NCCL version (need ≥ 2.26 for v3 + NetPlugin events)
python -c "import torch; print(torch.__version__, torch.cuda.nccl.version())"

# 2. Install probing (wheel includes libprobing_nccl_profiler.so on Linux)
pip install probing
# or from source: make wheel && pip install dist/probing-*.whl

# 3. Run a tiny distributed job
export NCCL_PROFILER_PLUGIN=$(python -m probing.nccl --plugin-path)
export NCCL_PROFILE_EVENT_MASK=$(python -m probing.nccl --event-mask)  # 26
export PROBING=2
torchrun --nproc_per_node=2 your_allreduce.py

# 4. Query (probing attached to same PIDs, or after inject)
probing -t <pid> query "SELECT * FROM nccl.proxy_ops LIMIT 20"
```

Optional NetPlugin (IB QP) events — add mask bit 128:

```bash
export NCCL_PROFILE_EVENT_MASK=154   # + ncclProfileNetPlugin(128)
probing -t <pid> query "SELECT * FROM nccl.net_qp LIMIT 20"
```

## Tables

| SQL table | mmap file | Phase |
|-----------|-----------|-------|
| `nccl.proxy_ops` | `nccl.proxy_ops` | wait decomposition + parallel roles |
| `nccl.net_qp` | `nccl.net_qp` | IB QP timing from NCCL net plugin |

### `nccl.proxy_ops` columns

| Column | Notes |
|--------|-------|
| `rank` | torch rank |
| `tp_rank`, `pp_rank`, `dp_rank` | from `TP_RANK` / `PP_RANK` / `DP_RANK` (or Megatron env); `-1` if unset |
| `send_gpu_wait_ns` | culprit signal |
| `recv_wait_ns` | victim signal |
| `coll_func`, `seq`, `channel_id`, `peer`, `is_send`, `n_steps`, `trans_bytes` | proxy op metadata |
| `send_wait_ns`, `recv_flush_wait_ns` | additional wait buckets |

Rows aggregate ProxyStep state transitions at ProxyOp stop (batched under parent Coll when present).

Docs: `docs/src/design/nccl-profiler.md`.

## Environment

| Variable | Purpose |
|----------|---------|
| `NCCL_PROFILER_PLUGIN` | Path to this `.so` (required) |
| `NCCL_PROFILE_EVENT_MASK` | Override default `Coll\|ProxyOp\|ProxyStep` (26) |
| `PROBING_DATA_DIR` | Memtable directory (default `/dev/shm/probing`) |
| `TP_RANK`, `PP_RANK`, `DP_RANK` | Parallel roles written into `nccl.proxy_ops` |
| `PROBING_NCCL_MOCK` | Dev mock tables (`auto` on macOS) — see `python/probing/nccl/mock.py` |

## Architecture

```
NCCL proxy thread
  → ncclProfiler_v3 callbacks (this crate)
  → slot pools (no per-event malloc)
  → aggregate ProxyStep waits into ProxyOp
  → batch rows under parent Coll; flush mmap on Coll stop
probing engine (same process)
  → MmapFileSchemaProvider discovers nccl.*
  → SELECT … FROM nccl.proxy_ops
```

### Phase 2 (writer path)

- Pre-allocated slot pools for Coll / ProxyOp / ProxyStep / NetPlugin
- ProxyOp rows batched under parent Coll; mmap flush on `Coll stopEvent`
- Orphan ProxyOps (no parent) flush immediately
- Write failures logged once to stderr; `finalize` reports `pool_exhausted` / `write_errors`

### Phase 3 (event hierarchy + roles)

- `parentObj` chain: Coll → ProxyOp (per channel send/recv) → ProxyStep
- `descr.rank` + Coll metadata propagated when parent is present
- `coll_func` stored in fixed buffer (no heap `String` on hot path)
- `tp_rank` / `pp_rank` / `dp_rank` from training env (`role.rs`, aligned with `python.probing.parallel`)

### Phase 4 (diagnostic skill)

- Skill `nccl_culprit_victim` in `skills/` — SQL rank wait summary, culprit/victim rankings, role-aligned view, `global.*` fan-out
- Linked from `slow_rank` and `comm_bottleneck` when `nccl.proxy_ops` is present

## Compatibility

- **Floor**: export `ncclProfiler_v3` only (NCCL 2.26+).
- NCCL 2.27+ may negotiate higher plugin versions if we add `ncclProfiler_v4`… later.
- PyTorch **2.8+** (NCCL 2.26+) recommended; 2.7 ships NCCL 2.25 (no v3).

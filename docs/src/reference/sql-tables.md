# SQL Tables

This page catalogs every built-in SQL table you can query through Probing. It's a
reference — if you're looking for query patterns and how-to, start with [SQL
Analytics](../guide/sql-analytics.md).

Each table is backed by an mmap ring buffer (MEMT) or registered dynamically by an
extension crate. Tables live under schema prefixes that reflect their data source:
`python.*` for training and Python runtime data, `cpu.*` / `gpu.*` for host and
device sampling, `cluster.*` for node registry, `nccl.*` for the NCCL profiler
plugin, and `global.<schema>.<table>` for federated cross-rank queries.

The authoritative schema definitions live in `probing/core/resources/tables.yaml` (agent
overlay) and in-code collector/`@table` docs (description SSOT). Query via
`probe.probing.table_docs` / `column_docs`. The tables on this page are kept in sync with
that overlay file.

To see what tables are actually available on a live endpoint:

```bash
probing $ENDPOINT tables
probing $ENDPOINT tables --all
```

Terminology: [Core Concepts](../guide/concepts.md) (endpoint, steps, `role`, federation).

## Schema prefixes

Each schema represents a category of data source. The tables listed below are organized
by these prefixes so you know where to look:

| Prefix | Data source |
|--------|-------------|
| `python.*` | Training and Python runtime (memtable-backed) |
| `cpu.*`, `gpu.*`, `process.*` | Host and device sampling (extension crates) |
| `cluster.*` | Cluster node registry |
| `nccl.*` | NCCL profiler plugin (optional, cdylib) |
| `global.<schema>.<table>` | Federated fan-out across registered peers |
| `information_schema.*` | Engine metadata and configuration |

## Federation

Tables with a **`global_name`** can be queried as `global.<path>` (e.g.
`global.python.comm_collective`). The master merges peer results and may attach:

| Tag | Description |
|-----|-------------|
| `_host` | Source hostname |
| `_addr` | Source probing HTTP address |
| `_rank` | `torch.distributed` rank (from `cluster.nodes`) |
| `_role` | Parallel role key (e.g. `dp=2,pp=1,tp=0`) |

Example:

```sql
SELECT _role, _rank, avg(duration_ms) AS avg_ms
FROM global.python.comm_collective
GROUP BY _role, _rank;
```

---

## Training & tracing (`python.*`)

### `python.torch_trace` {#python-torch_trace}

PyTorch module-level forward/step timings and GPU memory snapshots.

**Synonyms:** torch trace, module timing

| Column | Description |
|--------|-------------|
| `local_step` | Local training step (per rank) |
| `global_step` | Global step (`step_snapshot`) |
| `rank` | `torch.distributed` rank |
| `world_size` | World size |
| `role` | Parallel role key, e.g. `dp=2,pp=1,tp=0` |
| `seq` | Hook sequence within step |
| `module` | Fully-qualified module name |
| `stage` | `pre forward`, `post forward`, `pre step`, `post step` |
| `duration` | Hook duration (seconds); meaningful on post rows |
| `time_offset` | Seconds since step time anchor |
| `allocated` | GPU memory allocated (MB) |
| `allocated_delta` | Change in allocated since previous hook (MB) |
| `max_allocated` | Peak allocated (MB) |
| `max_allocated_delta` | Change in peak allocated (MB) |
| `cached` | GPU memory reserved (MB) |
| `max_cached` | Peak reserved (MB) |

**Notes:** First complete step is discovery-only (may have no rows). Backward hooks are off
by default.

---

### `python.comm_collective` {#python-comm_collective}

`torch.distributed` collective calls (all_reduce, broadcast, …).

**Synonyms:** collective, communication, NCCL, all_reduce

| Column | Description |
|--------|-------------|
| `local_step` | Local step on this rank |
| `global_step` | Global training step |
| `rank` | `torch.distributed` rank |
| `world_size` | World size |
| `role` | Parallel role key |
| `op` | Collective operation name |
| `group_rank` | Rank within process group |
| `group_size` | Process group size |
| `participate_ranks` | Participating ranks (serialized) |
| `tensor_shape` | Tensor shape string |
| `tensor_dtype` | Tensor dtype |
| `bytes` | Tensor bytes communicated |
| `duration_ms` | Wall time (milliseconds) |
| `async_op` | 1 if asynchronous collective |

**Global:** `global.python.comm_collective`
**Federation columns:** `_host`, `_addr`, `_rank`, `_role`

---

### `python.trace_event`

Span start/end and custom events (distributed tracing).

**Synonyms:** trace, span, timeline

| Column | Description |
|--------|-------------|
| `record_type` | `span_start` \| `span_end` \| `event` |
| `trace_id` | Trace id shared by related spans |
| `span_id` | Unique span id |
| `name` | Span or event name |
| `phase` | Training phase (`forward`, `backward`, `optimizer`) or empty |
| `time` | Timestamp (nanoseconds since epoch) |
| `attributes` | JSON metadata (rank, local_step, …) |

Join `span_start` / `span_end` on `span_id` for durations. See [Distributed](../design/distributed.md).

---

### `python.backtrace`

Live mixed Python + native stack (**point-in-time**, not a full history).

**Synonyms:** stack, backtrace, hang stack

| Column | Description |
|--------|-------------|
| `func` | Function name |
| `file` | Source file |
| `lineno` | Line number |
| `depth` | Stack depth (0 = innermost) |
| `frame_type` | `python` \| `native` |

Populate with `probing backtrace`, then `SELECT … FROM python.backtrace`.

---

### `python.variables`

Variable snapshots when variable tracing is enabled.

| Column | Description |
|--------|-------------|
| `micro_step` | Training micro-step |
| `func` | Function name |
| `name` | Variable name |
| `value` | String representation |

---

## System metrics

### `cpu.utilization`

Host CPU and RSS sampling (process and top threads).

| Column | Description |
|--------|-------------|
| `ts` | Sample timestamp (microseconds) |
| `scope` | `process` \| `thread` |
| `rss_kb` | Resident set size (KB) — process scope only |
| `cpu_total_pct` | CPU utilization (%) |
| `comm` | Thread/process name |
| `wchan` | Kernel wait channel (Linux) |

---

### `gpu.utilization`

GPU memory and utilization samples.

| Column | Description |
|--------|-------------|
| `ts` | Sample timestamp |
| `used_bytes` | Device memory used |
| `total_bytes` | Device memory total |
| `mem_used_pct` | Memory used (%) |
| `gpu_util_pct` | GPU compute utilization (-1 if unavailable) |

---

### `process.kmsg`

Linux kernel ring buffer (dmesg) — OOM killer, GPU Xid, IB errors. **Linux only.**

| Column | Description |
|--------|-------------|
| `timestamp` | Event time |
| `level` | Log level |
| `message` | Kernel message text |

---

## Cluster

### `cluster.nodes`

Registered distributed training peers (from `PUT /apis/nodes` / torchrun registration).

| Column | Description |
|--------|-------------|
| `host` | Hostname |
| `addr` | Probing HTTP address |
| `rank` | Global rank |
| `world_size` | World size |
| `local_rank` | Local rank on node |
| `role` | Parallel role key (source for federation `_role`) |
| `role_name` | Torchrun / Elastic role name (distinct from `role`) |
| `status` | Node status |
| `timestamp` | Last update (microseconds) |

```bash
probing -t <master> cluster nodes
# or: SELECT * FROM cluster.nodes
```

---

## NCCL profiler (optional)

Requires NCCL profiler plugin — see [NCCL Profiler](../design/nccl-profiler.md).

### `nccl.proxy_ops`

Per-proxy-op wait decomposition (culprit vs victim).

| Column | Description |
|--------|-------------|
| `ts` | Event timestamp (nanoseconds) |
| `rank` | `torch.distributed` rank |
| `tp_rank`, `pp_rank`, `dp_rank` | Parallel ranks (-1 if unknown) |
| `comm_hash` | NCCL communicator hash |
| `coll_func` | Collective name (AllReduce, AllGather, …) |
| `seq` | Collective sequence number |
| `channel_id` | NCCL channel id |
| `peer` | Peer rank for this proxy op |
| `is_send` | 1 if send proxy, 0 if recv |
| `n_steps` | ProxyStep count aggregated |
| `trans_bytes` | Bytes transferred |
| `send_gpu_wait_ns` | **Culprit** — local GPU not ready to send |
| `send_peer_wait_ns` | Waiting for receiver clear-to-send credits (v4 ABI only; 0 on v3) |
| `send_wait_ns` | Send-side network wait |
| `recv_wait_ns` | **Victim** — waiting on peer data |
| `recv_flush_wait_ns` | Recv flush wait |

**Global:** `global.nccl.proxy_ops`
**Federation columns:** `_host`, `_addr`, `_rank`, `_role`

---

### `nccl.net_qp`

NCCL NetPlugin IB QP completion timing (optional mask bit 128).

| Column | Description |
|--------|-------------|
| `ts` | Event timestamp (nanoseconds) |
| `rank` | `torch.distributed` rank |
| `device` | IB device index |
| `qp_num` | Queue pair number |
| `wr_id` | Work request id |
| `opcode` | IB opcode |
| `length` | Transfer length |
| `duration_ns` | QP completion duration |

**Global:** `global.nccl.net_qp`
**Federation columns:** `_host`, `_addr`, `_rank`, `_role`

---

## Metadata

### `information_schema.df_settings`

Runtime configuration key/value pairs (`probing.*` settings).

| Column | Description |
|--------|-------------|
| `name` | Setting name |
| `value` | Setting value |

---

## Custom tables

Plugins register `python.<name>` via `@table` dataclass. Schema is defined by the plugin
author — not listed here. See [Extensibility](../design/extensibility.md).

---

## Related

- [Core Concepts](../guide/concepts.md) — steps, role, `global.*`
- [SQL Analytics](../guide/sql-analytics.md) — query patterns
- [API Reference](../api-reference.md) — CLI and Python API

# How Probing Works

This page builds the mental model you need to use Probing effectively. It's not a
reference — it explains the architecture, the data flow, and the key design decisions
from a user's perspective. Read this before anything else.

## The two modes: in-process vs attach

Probing works in two fundamentally different ways. Understanding the difference avoids
a lot of confusion.

**In-process mode** is what you get with `PROBING=1 python train.py`. A `.pth` hook
fires at Python startup, and before your training script even runs its first line, an
embedded HTTP server is listening on a Unix socket and the Rust engine has registered
all its data sources. Your training code calls `import probing` and uses
`probing.query()` directly — no network, no subprocess.

```bash
PROBING=1 python train.py
```

**Attach mode** is what you get with `probing -t <pid> inject` — Linux only. The CLI
uses ptrace to load the probing shared library into an already-running process. The
process never restarted; it never imported probing. After injection, the same
in-process server starts inside the target.

```bash
probing -t $(pgrep -f train.py) inject
probing -t $(pgrep -f train.py) query "SELECT 1"
```

In both cases, the end result is the same: a running probing server inside the target
process. The CLI always talks to that server over HTTP (Unix socket for local PIDs,
TCP for remote `host:port`). There's no magic — `probing -t <pid> query "..."` is
literally an HTTP POST to `/query` on the embedded server.

On macOS and Windows, only in-process mode is available. The `inject` command is
Linux-only because it depends on ptrace.

## Where data comes from

Probing doesn't poll. Data is pushed into tables when events happen.

Every table you query is backed by one of two storage mechanisms:

**Mmap ring buffers (MEMT).** Most `python.*` tables and all extension tables use
this. A fixed-size memory-mapped file, structured as a ring of chunks. The writer
(the training process) appends rows to the current chunk. Readers (the SQL engine)
scan from any position. This is lock-free on the read path — the writer uses atomic
operations with Release/Acquire ordering, and readers never block the writer.

**Registered data sources (ProbeDataSource).** Some tables come from Rust code that
implements the `ProbeDataSource` trait. `python.backtrace` is a virtual table — it
captures the current stack on demand rather than storing a history. `process.envs`
reads the process environment. `gpu.devices` enumerates CUDA devices.

The distinction matters because it affects what you can query:

| Table | Mechanism | What you get |
|---|---|---|
| `python.torch_trace` | mmap ring buffer | History of all hook invocations since sampling started |
| `python.comm_collective` | mmap ring buffer | History of all collectives since sampling started |
| `python.backtrace` | Virtual table | **Current** stack only — no history |
| `cpu.utilization` | mmap ring buffer | Time-series of CPU samples |
| `process.envs` | Virtual table | Current environment variables |

This is why `backtrace` requires a separate command (`probing backtrace`) — it
captures the stack into the virtual table, and then you query it. The table doesn't
accumulate data on its own.

## How tables are organized

All tables live under a schema prefix that tells you where the data came from:

`python.*`
: Training semantics — `torch_trace` (module hooks), `comm_collective` (distributed
ops), `trace_event` (spans), `backtrace` (stacks), `variables` (watched values).
Plus any custom tables you create with `@table`.

`cpu.*` / `gpu.*`
: Host and device sampling. `cpu.utilization` has per-process and per-thread CPU/RSS
samples. `gpu.utilization` has GPU memory and compute utilization.

`cluster.*`
: Cluster node registry. `cluster.nodes` lists every peer that has registered via
torchrun or the HTTP API.

`nccl.*`
: NCCL profiler plugin output. `nccl.proxy_ops` decomposes collective wait time into
culprit (local GPU not ready) and victim (waiting on peer data) components.

`global.<schema>.<table>`
: Federation. Prefix any table with `global.` to fan out the query to all registered
cluster peers. The master merges results and attaches `_host`, `_addr`, `_rank`,
`_role` tags to each row.

The full column reference for every built-in table is at [SQL Tables](../reference/sql-tables.md).

## Step coordinates: the shared time axis

Timestamps are unreliable for training analysis — steps are deterministic and align
naturally with training semantics. Probing uses a three-level step coordinate system:

`micro_step` is the finest counter. Increments each time `probing.step()` is called.
`local_step` is the optimizer step — `micro_step // micro_batches`. With gradient
accumulation of 10, every 10 micro-steps produce one local step.
`global_step` is the cluster-wide step, equal to `local_step` when ranks are aligned.

```python
probing.step(micro_batches=10)   # gradient accumulation factor
probing.step()                   # micro_step += 1 at each micro-batch boundary

# Later, in queries:
# SELECT local_step, AVG(duration) FROM python.torch_trace GROUP BY local_step
```

Every training-related table (`torch_trace`, `comm_collective`) carries both
`local_step` and `global_step` columns. The step coordinates are managed in Rust
(not a Python counter) so they're consistent even if the training script's state
gets corrupted.

## Role: encoding parallel topology in one column

Distributed training decomposes work across multiple parallelism dimensions —
tensor parallel (TP), pipeline parallel (PP), data parallel (DP), expert parallel
(EP), and combinations of them. Probing encodes the entire topology as a compact
sorted string: `dp=2,pp=1,tp=0`.

This string is stamped on every `torch_trace` and `comm_collective` row. Because
it's a single column, you can GROUP BY role in SQL to compare performance across
parallelism dimensions:

```sql
SELECT role, AVG(duration_ms) as avg_ms
FROM python.comm_collective
GROUP BY role;
```

Set it from environment variables (`PROBING_TP_RANK`, `PROBING_TP_SIZE`, etc., or
Megatron-style `TP_RANK`/`PP_RANK`/`DP_RANK`) or from Python:

```python
probing.set_role(dp=2, pp=1, tp=0)
# or: probing.set_role("dp=2,pp=1,tp=0")
probing.clear_role()  # fall back to environment-derived role
```

Note: `cluster.nodes` has `role_name` and `role_rank` — those are torchrun/Elastic
launcher fields describing the launcher role, not the parallel topology. Probing's
`role` is the parallelism key for analytics; they serve different purposes.

## Federation: querying across the cluster

When multiple training ranks are registered in a cluster, prefixing a table with
`global.` fans out the SQL query to every registered peer. The query runs
independently on each node; the master collects and concatenates the results.

Each returned row gets four federation tags added at query time:

| Tag | Source |
|-----|--------|
| `_host` | Hostname of the node that produced the row |
| `_addr` | That node's probing `host:port` |
| `_rank` | `torch.distributed` rank from the cluster node registry |
| `_role` | Parallel role key from the node registry |

A query like this:

```sql
SELECT _rank, op, AVG(duration_ms) as avg_ms
FROM global.python.comm_collective
WHERE local_step > 100
GROUP BY _rank, op
ORDER BY avg_ms DESC;
```

...runs on every registered rank, then the master merges all results into one result
set with `_rank` telling you which row came from where.

Nodes register via torchrun (Rust ctor starts cluster heartbeat by default — see
[torchrun cluster heartbeat](../design/torchrun-cluster.md)) or by PUTting to
`/apis/nodes`. Check current registration with `probing -t <master> cluster nodes`.

The `_role` tag uses the value from the **node registry**, which is kept in sync
with calls to `set_role()`. The `role` column on individual data rows uses the value
at **write time**. In practice these are the same, but understanding the distinction
matters when diagnosing stale role data.

## Extension paths: three ways to add capability

Probing has three distinct extension mechanisms. They're not interchangeable — each
serves a different purpose:

**1. Data table plugin (`@table` in Python).**
Define a dataclass, decorate it, append rows from your code. The table appears as
`python.<name>` and is immediately queryable. Use this when you have new metrics or
events to record. Built on mmap ring buffers.

**2. Diagnostic skill (`steps.yaml` + `SKILL.md`).**
A YAML workflow that runs SQL queries against existing tables, applies interpretation
rules, and produces findings. Use this when you have a diagnosis recipe to codify —
like "find the slowest rank" or "check for NCCL wait imbalance." Run with
`probing skill run <id>`. Skills don't collect new data; they analyze existing data.

**3. Rust extension (`ProbeExtension` + `ProbeDataSource`).**
A compiled Rust crate that registers new data sources (virtual tables, mmap tables)
and/or configurable options with side effects (like starting a CPU sampler). Use
this when you need system-level access — ptrace, CUDA APIs, RDMA counters. The NCCL
profiler is a variant of this: a C ABI plugin loaded by NCCL itself, not by Probing.

See [Extensibility](../design/extensibility.md) for the full development guide.

## What happens when things go wrong

Knowing what to expect when a query fails saves debugging time.

**Invalid SQL** returns a `PyRuntimeError` with the DataFusion error message. The
error usually tells you exactly what's wrong (unknown table, unknown column, syntax
error). If you get a cryptic DataFusion error, check the server logs — set
`PROBING_LOGLEVEL=debug` for verbose output.

**Missing tables.** If `probing $ENDPOINT tables` doesn't show the table you expect,
the data source isn't active. Common causes: the GPU extension isn't compiled in
(check `probing $ENDPOINT query "SELECT name FROM information_schema.df_settings WHERE name LIKE 'probing.gpu%'"`), or the mmap file isn't being written to (check
`$PROBING_DATA_DIR/<pid>/`).

**Empty results from `python.torch_trace`.** The PyTorch profiler needs
`PROBING_TORCH_PROFILING=on` at startup. Without it, hooks are never registered and
no rows are written.

**Injection fails with ESRCH.** On Linux, ptrace may be restricted by YAMA LSM
(`/proc/sys/kernel/yama/ptrace_scope`). Set it to 0 or run as the same user.

See [Troubleshooting](troubleshooting.md) for more.

## Environment variables at a glance

Probing has many configuration points. The most important ones:

| Variable | Effect |
|----------|--------|
| `PROBING` | `0`=disabled, `1`=current process, `2`=current+children, `regex:...`=pattern match |
| `PROBING_TORCH_PROFILING` | `on` to activate PyTorch module hooks |
| `PROBING_DATA_DIR` | Where mmap ring buffers are stored |
| `PROBING_PORT` | TCP port for remote access (or `RANDOM`) |
| `PROBING_AUTH_TOKEN` | Authentication token for remote mode |
| `PROBING_CPU_SAMPLE_MS` | CPU sampling interval in milliseconds (0=off) |
| `PROBING_GPU_SAMPLE_MS` | GPU sampling interval in milliseconds |
| `PROBING_SPAN_BACKENDS` | Comma-separated: `memtable`, `logger`, `otel` |
| `PROBING_LOGLEVEL` | `trace`, `debug`, `info`, `warn`, `error` |

The complete reference is at [Environment Variables](../reference/env-vars.md).

# Core Concepts

One-page glossary for terms used across tutorials, guides, and design docs.
When in doubt, start here before diving into [SQL Analytics](sql-analytics.md) or
[Distributed](../design/distributed.md).

## 1. Endpoint

Every **CLI** command targets a running probing server via an **endpoint**:

| Form | Example | Notes |
|------|---------|-------|
| Local PID | `12345` | `probing -t 12345 query "…"` |
| Host:port | `node-a:8080` | Remote TCP; set `PROBING_PORT` at training startup |

```bash
export ENDPOINT=12345   # or host:8080
probing $ENDPOINT query "SELECT 1"
```

**In-process** (training script): set `PROBING=1` (or inject on Linux) and `import probing`
— no endpoint string; use `probing.query()` directly.

There is **no** `probing.connect()` Python API. Remote access is always CLI `-t <endpoint>`.

---

## 2. Three CLI commands

| Command | Usage | Data |
|---------|-------|------|
| **query** | `probing $ENDPOINT query "<sql>"` | Recorded table rows |
| **eval** | `probing $ENDPOINT eval "<code>"` | One-off Python in the target process |
| **backtrace** | `probing $ENDPOINT backtrace` | Point-in-time stack → `python.backtrace` |

These are the main **CLI entry points** from outside the process — not the full product
surface (continuous profiling, federation, `global.*`, skills, etc. are covered below).
Typical flow: `backtrace` captures state → `eval` inspects live objects → `query` analyzes history.

---

## 3. Data tables (`python.*`)

Probe data lives in **append-only SQL tables** under the `python` schema (plus built-in
extensions like `cpu.utilization`, `cluster.nodes`, `nccl.proxy_ops`).

| Table | What it records |
|-------|-----------------|
| `python.torch_trace` | Module hook timings + GPU memory |
| `python.comm_collective` | `torch.distributed` collective wall time |
| `python.trace_event` | Span start/end and custom events |
| `python.backtrace` | Latest captured stack (not a full history) |
| `python.variables` | Watched variable snapshots (when enabled) |

Custom plugins use the same model: `@table` dataclass + `.save()` → `python.<name>`.
Column reference: **[SQL Tables](../reference/sql-tables.md)**.

Tables are **not** lazy snapshots — rows are pushed when events happen (hook, collective,
span end).

---

## 4. Step coordinates

Training analysis needs a **shared step index**. Probing uses Rust `step_snapshot()` as the
single source of truth (not a separate Python counter).

| Field | Meaning |
|-------|---------|
| `local_step` | Per-rank step counter (optimizer-step aligned) |
| `global_step` | Cluster-wide step (when coordinated) |

On data rows:

- `python.torch_trace.step` → local step
- `python.torch_trace.global_step`, `python.comm_collective.local_step` / `global_step`

In-process:

```python
from probing.tracing import step_snapshot
s = step_snapshot()
print(s.local_step, s.global_step, s.rank)
```

Prefer these fields in SQL and skills — not `trainer.current_step`.

---

## 5. Parallel role

Distributed training places each process in a **parallel topology** (TP / PP / DP / EP / …).
Probing encodes this as one extensible string **`role`**, not one column per dimension.

**Format:** sorted `name=value` pairs, e.g. `dp=2,pp=1,tp=0`. Empty string when unset.

| Source | How |
|--------|-----|
| Environment | Megatron-style `*_PARALLEL_RANK`, or `PROBING_ROLE_<NAME>=<int>` |
| Runtime | `probing.set_role("dp=2,pp=1,tp=0")` or `set_role(dp=2, pp=1)` |
| Read | `probing.current_role()`; `clear_role()` reverts to env |

`role` is stamped on **`python.torch_trace`** and **`python.comm_collective`** rows so you
can `JOIN` / `GROUP BY role` across tables on one rank.

Distinct from torchrun's **`role_name`** / `role_rank` on `cluster.nodes` — those are
Elastic/job launcher fields. Probing's `role` is the parallel-placement key for analytics.

---

## 6. Federation (`global.*` and tags)

For **multi-rank** SQL, use the `global` catalog: `global.python.comm_collective` fans out
to registered peers and merges results.

Each row gets **federation tags** identifying the source probing endpoint:

| Tag | Meaning |
|-----|---------|
| `_host` | Source hostname |
| `_addr` | Source `host:port` |
| `_rank` | `torch.distributed` rank (from node registry) |
| `_role` | Parallel role key (from node registry / `set_role`) |

Example:

```sql
SELECT _role, _rank, avg(duration_ms) AS avg_ms
FROM global.python.comm_collective
WHERE global_step > 100
GROUP BY _role, _rank
ORDER BY avg_ms DESC;
```

Register nodes via torchrun (`setup_torchrun_cluster`) or `PUT /apis/nodes`. CLI:
`probing -t <master> cluster nodes` / `cluster query "…"`. Details:
[Distributed](../design/distributed.md).

Row column `role` = value at **write time** on that rank. Tag `_role` = value on the
**node registry** at federation time (kept in sync via `set_role` + re-register).

---

## 7. Data plugin vs diagnostic skill

| | **Table plugin** (Path 1) | **Diagnostic skill** (Path 2) |
|--|---------------------------|----------------------------------|
| You add | Dataclass table + rows | `SKILL.md` + optional `steps.yaml` |
| Output | `python.my_table` | Findings + SQL steps / agent guidance |
| Run | `SELECT …` | `probing skill run <id>` |
| Use when | New **metrics/events** to store | New **investigation recipe** |

Optional **Path 3**: NCCL profiler cdylib → `nccl.proxy_ops` for culprit/victim wait
decomposition. See [Extensibility](../design/extensibility.md).

---

## Where to go next

| Goal | Doc |
|------|-----|
| SQL patterns | [SQL Analytics](sql-analytics.md) |
| Table schemas | [SQL Tables](../reference/sql-tables.md) |
| Multi-node | [Distributed](../design/distributed.md) |
| Write a plugin | [Extensibility](../design/extensibility.md) |
| CLI / Python API | [API Reference](../api-reference.md) |

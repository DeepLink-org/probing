# Hierarchical cluster query aggregation

Cross-rank **`cluster query`** and **`global.*`** federation default to **hierarchical fan-out** so the coordinator (usually global rank 0) does not open one HTTP connection per training rank at wan scale.

Aligns with [Torchrun cluster heartbeat](torchrun-cluster.md) membership tiers. SQL semantics: [Federated query engine](federation.md).

---

## 1. Cost model

Flat fan-out sends concurrent HTTP from the coordinator to **every** live peer in `cluster.nodes`:

- Wan scale ≈ **O(world_size)** concurrent connections (e.g. 8192–10240)
- Rank-0 coordinator memory and socket pressure
- One slow rank bounds total latency (≈ slowest peer)

Hierarchical fan-out splits the query into **coordinator → per-machine local0 → on-node leaf ranks**. Coordinator-side connections ≈ **O(number of nodes)**.

---

## 2. Tiers

```text
coordinator (global rank 0 / query entry, local_rank=0)
  │
  ├─ Local node tier (scope=node)
  │     local0 executes SQL locally
  │     └─ fan-out → leaf ranks on same group_rank (POST /query, local only)
  │     └─ merge rows / aggregate partials → node result
  │
  └─ Remote node tier (scope=coordinator → each machine local0)
        POST /apis/cluster/query  { scope: "node", ... }
        each local0 repeats the local node tier, returns to coordinator
        coordinator merges node partials + injects federation tags
```

| Tier | Who | Fan-out targets | Example (8 GPUs/node, 1024 nodes) |
|------|-----|-----------------|-------------------------------------|
| **Coordinator** | rank0 probe | Each machine `local_rank=0` (one per `group_rank`) | ~1023 remote nodes |
| **Node** | Each machine local0 | Leaf ranks on same machine | ~7 / node |
| **Leaf** | `local_rank>0` | None (local execute only) | — |

---

## 3. Enable and disable

### Default

- **`PROBING_CLUSTER_FANOUT_HIERARCHICAL=1`** (on by default)
- `POST /apis/cluster/query` and CLI `probing cluster query` default **`hierarchical: true`**

### Disable (flat fan-out)

```bash
export PROBING_CLUSTER_FANOUT_HIERARCHICAL=0
# Or per request
probing -t rank0:8080 cluster query --flat "SELECT ..."
```

```json
POST /apis/cluster/query
{ "expr": "...", "cluster": true, "hierarchical": false }
```

### Prerequisites

Hierarchical mode depends on metadata in `cluster.nodes`:

| Field | Purpose |
|-------|---------|
| `group_rank` / `NODE_RANK` | Physical node identity |
| `local_rank` | Distinguish local0 (`0`) vs leaf |
| `addr` | HTTP fan-out target |

Filled automatically by torchrun heartbeat / `PUT /apis/nodes`. If the cluster view **lacks** these fields while hierarchical mode is on (default), **`POST /apis/cluster/query` returns HTTP 503** — probing does **not** silently fall back to flat fan-out. Use `hierarchical=false` (or `--flat`) only when you explicitly accept flat fan-out.

---

## 4. API

### `POST /apis/cluster/query`

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `expr` | string | — | SQL |
| `cluster` | bool | `false` | Cross-node query |
| `hierarchical` | bool | `true` | Use hierarchical fan-out |
| `scope` | string | `auto` | `auto` / `coordinator` / `node` / `local` |

**`scope` values**

| Value | Behavior |
|-------|----------|
| `auto` | local0 entry → `coordinator`; leaf → `local` |
| `coordinator` | Local node aggregation + remote node aggregators |
| `node` | This machine only: local0 + leaves (called by coordinator) |
| `local` | Current process only, no fan-out |

### Response `meta`

```json
{
  "cluster": true,
  "hierarchical": true,
  "scope": "coordinator",
  "nodes_queried": 3,
  "nodes_failed": [],
  "peer_batches_dropped": 0,
  "partial": false,
  "node_aggregators_queried": 1,
  "local_ranks_queried": 1
}
```

| Field | Meaning |
|-------|---------|
| `partial` | `true` when any peer failed or merge dropped batches — HTTP **503** with partial `dataframe` (unless `PROBING_FANOUT_STRICT=1`, then the query fails entirely) |
| `peer_batches_dropped` | Partial peer DataFrames dropped during coordinator merge |
| `nodes_queried` | Successful **HTTP endpoints** in this query (local local0, local leaves, remote node aggs) |
| `node_aggregators_queried` | Remote **local0** endpoints contacted at coordinator tier |
| `local_ranks_queried` | **Leaf ranks** contacted on the coordinator machine |
| `nodes_failed` | Peers that timed out or returned HTTP errors |

!!! note "Not world_size"
    `nodes_queried` counts **HTTP endpoints**, not torch ranks. A 2-node × 2-GPU hierarchical query is typically `3` (2 local endpoints + 1 remote node agg), not `4`.

### CLI

```bash
# Default: hierarchical
probing -t rank0:8080 cluster query "
  SELECT _rank, avg(duration_ms) AS avg_ms
  FROM global.python.comm_collective
  GROUP BY _rank
  ORDER BY avg_ms DESC
  LIMIT 10
"

# Flat (avoid at wan scale)
probing -t rank0:8080 cluster query --flat "SELECT ..."
```

### Web

`GET /apis/training/step_matrix?cluster=true` uses hierarchical fan-out by default.

---

## 5. Relationship to federation paths

| Federation path | Hierarchical behavior |
|-----------------|----------------------|
| **A — aggregate pushdown** | Coordinator sends `per_node_sql` to **node aggregators**; local machine also fans out to leaves; see `aggregate_pushdown.rs` |
| **C — broadcast** (JOIN / CTE) | Coordinator runs node aggregation locally; remote nodes recurse via `scope=node` |
| **B — federated scan** | Remote lazy partitions under `FanoutScope::Coordinator` pull **node aggregators** only |

Complex CTE + window queries should still be split into diagnostic chains (see [Federated query engine §4.7](federation.md#path-c-broadcast)).

---

## 6. Environment variables

| Variable | Default | Description |
|----------|---------|-------------|
| `PROBING_CLUSTER_FANOUT_HIERARCHICAL` | `1` | `0` = global flat fan-out (legacy O(world_size) path) |
| `PROBING_REMOTE_QUERY_TIMEOUT_SECS` | `30` | Per-peer HTTP timeout (per tier in hierarchical mode); see [Environment variables](../reference/env-vars.md) |
| `PROBING_FANOUT_STRICT` | unset | When `1` or `true`, any peer failure or dropped batch fails the whole query (no partial 503) |

When hierarchical mode is on (default) but `cluster.nodes` lacks `group_rank` / `local_rank` (heartbeat not converged), **`POST /apis/cluster/query` returns HTTP 503** instead of silently falling back to flat fan-out. Use `hierarchical=false` only when you explicitly accept flat fan-out.

Cluster heartbeat variables: [Environment variables — cluster](../reference/env-vars.md) and [Torchrun cluster heartbeat](torchrun-cluster.md).

---

## 7. Implementation

| Module | Path |
|--------|------|
| Fan-out orchestration | `probing/server/src/server/cluster_fanout.rs` |
| HTTP handler | `probing/server/src/server/cluster_query.rs` |
| Peer selection | `probing/core/src/core/cluster.rs` (`node_aggregator_peers`, `local_leaf_peers`) |
| Fan-out scope | `probing/core/src/core/federation/fanout_scope.rs` |
| Remote execution | `probing/core/src/core/federation/cluster_executor.rs` |

Integration test: `tests/regression/rust/probing/server/hierarchical_fanout_query.rs` (`server_hierarchical_fanout_query`).

---

## 8. Related

| Document | Content |
|----------|---------|
| [Distributed overview](distributed.md) | `cluster nodes` / `cluster query` |
| [Federated query engine](federation.md) | `global.*`, diagnostic SQL, wan-scale bar |
| [Torchrun cluster heartbeat](torchrun-cluster.md) | Membership tiers |
| [Modularity — cross-rank fan-out](modularity.md) | L3 control-plane ownership |

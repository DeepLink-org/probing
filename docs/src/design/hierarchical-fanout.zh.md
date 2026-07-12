# 分层集群查询聚合

跨 rank 的 `cluster query` / `global.*` 查询默认走 **分层 fan-out**，避免 coordinator（通常是 global rank 0）在万卡规模下直连每一个训练 rank。

与 [torchrun 集群心跳](torchrun-cluster.zh.md) 的成员注册层级对齐；联邦 SQL 语义见 [联邦查询引擎](federation.zh.md)。

---

## 1. 代价模型

扁平 fan-out 从 coordinator 向 `cluster.nodes` 中 **每个** 存活 peer 并发 HTTP：

- 万卡 ≈ **O(world_size)** 并发连接（例如 8192～10240）
- 单点 rank0 内存与 socket 压力大
- 慢 rank 拖垮整次查询（总延迟 ≈ 最慢 peer）

分层 fan-out 将查询拆成 **coordinator → 各机 local0 → 同机 leaf rank**，coordinator 侧连接数 ≈ **O(节点数)**。

---

## 2. 层级

```text
coordinator (global rank 0 / 查询入口, local_rank=0)
  │
  ├─ 本机 node 层 (scope=node)
  │     local0 本地执行 SQL
  │     └─ fan-out → 同 group_rank 的 leaf ranks (POST /query, 仅本地)
  │     └─ 行合并 / 聚合 partial → 本机结果
  │
  └─ 远程 node 层 (scope=coordinator → 各机 local0)
        POST /apis/cluster/query  { scope: "node", ... }
        各机 local0 重复「本机 node 层」逻辑后返回
        coordinator 合并各 node partial + 注入联邦标签
```

| 层级 | 谁 |  fan-out 目标 | 典型规模 (8 卡/机, 1024 机) |
|------|-----|---------------|------------------------------|
| **Coordinator** | rank0 探针 | 各机 `local_rank=0`（每 `group_rank` 一个） | ~1023 远程 node |
| **Node** | 各机 local0 | 同机 leaf ranks | ~7 / node |
| **Leaf** | `local_rank>0` | 无（仅本地执行） | — |

---

## 3. 启用与关闭

### 默认

- **`PROBING_CLUSTER_FANOUT_HIERARCHICAL=1`**（默认开启）
- `POST /apis/cluster/query` 与 CLI `probing cluster query` 的 **`hierarchical` 默认为 `true`**

### 关闭（恢复扁平 fan-out）

```bash
export PROBING_CLUSTER_FANOUT_HIERARCHICAL=0
# 或单次请求
probing -t rank0:8080 cluster query --flat "SELECT ..."
```

```json
POST /apis/cluster/query
{ "expr": "...", "cluster": true, "hierarchical": false }
```

### 前置条件

分层依赖 `cluster.nodes` 中的元数据：

| 字段 | 用途 |
|------|------|
| `group_rank` / `NODE_RANK` | 区分物理节点 |
| `local_rank` | 识别 local0（`0`）与 leaf |
| `addr` | HTTP fan-out 目标 |

由 torchrun 心跳 / `PUT /apis/nodes` 自动填充。若集群视图 **缺少** 上述字段且分层模式开启（默认），**`POST /apis/cluster/query` 返回 HTTP 503** — 不会静默降级为扁平 fan-out。仅在明确接受扁平 fan-out 时使用 `hierarchical=false`。

---

## 4. API

### `POST /apis/cluster/query`

| 字段 | 类型 | 默认 | 说明 |
|------|------|------|------|
| `expr` | string | — | SQL |
| `cluster` | bool | `false` | 是否跨节点 |
| `hierarchical` | bool | `true` | 是否分层 fan-out |
| `scope` | string | `auto` | `auto` / `coordinator` / `node` / `local` |

**`scope` 含义**

| 值 | 行为 |
|----|------|
| `auto` | local0 入口 → `coordinator`；leaf → `local` |
| `coordinator` | 本机 node 聚合 + 远程 node aggregators |
| `node` | 仅本机 local0 + leaf（供 coordinator 调用） |
| `local` | 仅当前进程，不 fan-out |

### 响应 `meta`

```json
{
  "cluster": true,
  "hierarchical": true,
  "scope": "coordinator",
  "nodes_queried": 3,
  "nodes_failed": [],
  "node_aggregators_queried": 1,
  "local_ranks_queried": 1
}
```

| 字段 | 含义 |
|------|------|
| `nodes_queried` | 成功参与本次查询的 **endpoint 数**（含本机 local0、本机 leaf、远程 node agg） |
| `node_aggregators_queried` | coordinator 层联系的远程 **local0** 数量 |
| `local_ranks_queried` | 本机 node 层联系的 **leaf rank** 数量 |
| `nodes_failed` | 超时或 HTTP 失败的 peer 地址 |

!!! note "不是 world_size"
    `nodes_queried` 统计的是 **HTTP endpoint**，不是 torch rank 总数。2 机 × 2 卡分层查询通常为 `3`（本机 2 endpoint + 1 远程 node agg），而非 `4`。

### CLI

```bash
# 默认分层
probing -t rank0:8080 cluster query "
  SELECT _rank, avg(duration_ms) AS avg_ms
  FROM global.python.comm_collective
  GROUP BY _rank
  ORDER BY avg_ms DESC
  LIMIT 10
"

# 扁平（万卡慎用）
probing -t rank0:8080 cluster query --flat "SELECT ..."
```

### Web

`GET /apis/training/step_matrix?cluster=true` 默认走分层 fan-out。

---

## 5. 与联邦路径的关系

| 联邦路径 | 分层行为 |
|----------|----------|
| **A 聚合下推** | coordinator 向 **node aggregators** 发送 `per_node_sql`；本机额外 fan-out 至 leaf；见 `aggregate_pushdown.rs` |
| **C broadcast**（JOIN / CTE） | coordinator 对本机做 node 聚合，远程 node 通过 `scope=node` 递归 |
| **B 联邦 scan** | 远程 lazy 分区在 `FanoutScope::Coordinator` 下仅拉 node aggregator |

复杂 CTE + 窗口仍建议拆诊断链（见 [联邦查询引擎 §4.7](federation.zh.md#47-路径-c-broadcast)）。

---

## 6. 环境变量

| 变量 | 默认 | 说明 |
|------|------|------|
| `PROBING_CLUSTER_FANOUT_HIERARCHICAL` | `1` | `0` = 全局扁平 fan-out（legacy O(world_size) 路径） |
| `PROBING_REMOTE_QUERY_TIMEOUT_SECS` | `30` | 单 peer HTTP 超时（分层下为 per-tier）；见 [环境变量](../reference/env-vars.zh.md) |
| `PROBING_FANOUT_STRICT` | unset | `1`/`true` 时任一 peer 失败或 batch 丢弃则整查失败（无 partial 503） |

分层模式开启（默认）但 `cluster.nodes` 缺少 `group_rank` / `local_rank`（心跳未收敛）时，**`POST /apis/cluster/query` 返回 HTTP 503**，不再静默回退扁平 fan-out。仅在明确接受扁平 fan-out 时使用 `hierarchical=false`。

集群心跳相关变量见 [环境变量 — 集群](../reference/env-vars.zh.md#集群) 与 [torchrun 集群心跳](torchrun-cluster.zh.md)。

---

## 7. 实现位置

| 模块 | 路径 |
|------|------|
| Fan-out 编排 | `probing/server/src/server/cluster_fanout.rs` |
| HTTP handler | `probing/server/src/server/cluster_query.rs` |
| Peer 选择 | `probing/core/src/core/cluster.rs`（`node_aggregator_peers`, `local_leaf_peers`） |
| Fan-out scope | `probing/core/src/core/federation/fanout_scope.rs` |
| 远程执行 | `probing/core/src/core/federation/cluster_executor.rs` |

集成测试：`tests/regression/rust/probing/server/hierarchical_fanout_query.rs`（`server_hierarchical_fanout_query`）。

---

## 8. 相关文档

| 文档 | 内容 |
|------|------|
| [分布式架构](distributed.zh.md) | `cluster nodes` / `cluster query` |
| [联邦查询引擎](federation.zh.md) | `global.*`、诊断 SQL、万卡五连 |
| [torchrun 集群心跳](torchrun-cluster.zh.md) | 成员注册层级 |
| [模块化 — 跨 rank fan-out](modularity.zh.md) | L3 控制面职责 |

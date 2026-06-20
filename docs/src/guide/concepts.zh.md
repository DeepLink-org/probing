# 核心概念

本页是教程、指南与设计文档共用的**术语锚点**。建议先读这里，再进入
[SQL 分析](sql-analytics.zh.md) 或 [分布式](../design/distributed.zh.md)。

## 1. Endpoint（端点）

**CLI** 通过 **endpoint** 连接目标进程上的 probing 服务：

| 形式 | 示例 | 说明 |
|------|------|------|
| 本地 PID | `12345` | `probing -t 12345 query "…"` |
| host:port | `node-a:8080` | 远程 TCP；训练启动时设 `PROBING_PORT` |

```bash
export ENDPOINT=12345   # 或 host:8080
probing $ENDPOINT query "SELECT 1"
```

**进程内**（训练脚本）：`PROBING=1`（或 Linux 上 inject）后 `import probing`，直接用
`probing.query()`，无需 endpoint 字符串。

**没有** `probing.connect()`；访问远程进程一律用 CLI `-t <endpoint>`。

---

## 2. 三种 CLI 命令

| 命令 | 用法 | 数据 |
|------|------|------|
| **query** | `probing $ENDPOINT query "<sql>"` | 已采集的表数据 |
| **eval** | `probing $ENDPOINT eval "<code>"` | 在目标进程执行一次性 Python |
| **backtrace** | `probing $ENDPOINT backtrace` | 瞬时栈 → `python.backtrace` |

它们是进程外的**主要 CLI 入口**，不等于 Probing 的全部能力（持续采集、联邦、`global.*`、
skill 等见下文各节）。典型组合：`backtrace` 抓现场 → `eval` 看 live 对象 → `query` 做历史分析。

---

## 3. 数据表（`python.*`）

探针数据在 **`python` schema** 下的 **append-only** SQL 表中（另有 `cpu.utilization`、
`cluster.nodes`、`nccl.proxy_ops` 等内置/扩展表）。

| 表 | 记录内容 |
|----|----------|
| `python.torch_trace` | 模块 hook 耗时 + GPU 显存 |
| `python.comm_collective` | `torch.distributed` collective 墙钟时间 |
| `python.trace_event` | Span 起止与自定义事件 |
| `python.backtrace` | 最近一次捕获的栈（非全历史） |
| `python.variables` | 监视变量快照（启用时） |

自定义插件同样：`@table` dataclass + `.save()` → `python.<表名>`。
列说明见 **[SQL 表目录](../reference/sql-tables.zh.md)**。

表**不是**懒加载快照——事件发生时推送行（hook、collective、span 结束等）。

---

## 4. Step 坐标

训练分析需要统一的 **step 索引**。权威来源是 Rust `step_snapshot()`（不是单独的 Python 计数器）。

| 字段 | 含义 |
|------|------|
| `local_step` | 每 rank 本地步（与 optimizer step 对齐） |
| `global_step` | 集群全局步（协调时） |

数据行上：

- `python.torch_trace.step` → 本地步
- `python.torch_trace.global_step`、`python.comm_collective` 的 `local_step` / `global_step`

进程内：

```python
from probing.tracing import step_snapshot
s = step_snapshot()
print(s.local_step, s.global_step, s.rank)
```

SQL 与 skill 请用上述字段，**不要**用 `trainer.current_step`。

---

## 5. Parallel role（并行角色）

分布式训练里每个进程有并行拓扑（TP / PP / DP / EP / …）。Probing 用可扩展字符串 **`role`**
表示，而不是每种维度一列。

**格式：** 按名字排序的 `name=value`，如 `dp=2,pp=1,tp=0`；未设置时为 `""`。

| 来源 | 方式 |
|------|------|
| 环境变量 | Megatron 风格 `*_PARALLEL_RANK`，或 `PROBING_ROLE_<NAME>=<int>` |
| 运行时 | `probing.set_role("dp=2,pp=1,tp=0")` 或 `set_role(dp=2, pp=1)` |
| 读取 | `probing.current_role()`；`clear_role()` 恢复 env 推导 |

`role` 写入 **`python.torch_trace`**、**`python.comm_collective`**，便于同 rank 上按 role
JOIN / GROUP BY。

与 torchrun 的 **`role_name`** / `role_rank`（`cluster.nodes` 上）不同——那是 Elastic 作业
字段；Probing 的 `role` 是**并行放置 key**，用于分析对齐。

---

## 6. 联邦查询（`global.*` 与标签）

跨 rank SQL 使用 **`global` catalog**，例如 `global.python.comm_collective`：向已注册节点
fan-out 后合并。

每行附加**联邦标签**，标识数据来源：

| 标签 | 含义 |
|------|------|
| `_host` | 来源主机名 |
| `_addr` | 来源 `host:port` |
| `_rank` | `torch.distributed` rank（节点注册表） |
| `_role` | 并行角色 key（注册表 / `set_role`） |

示例：

```sql
SELECT _role, _rank, avg(duration_ms) AS avg_ms
FROM global.python.comm_collective
WHERE global_step > 100
GROUP BY _role, _rank
ORDER BY avg_ms DESC;
```

节点通过 torchrun（`setup_torchrun_cluster`）或 `PUT /apis/nodes` 注册。CLI：
`probing -t <master> cluster nodes` / `cluster query "…"`。详见
[分布式](../design/distributed.zh.md)。

行内列 **`role`** = 该 rank **写入时**的值；标签 **`_role`** = **联邦查询时**节点注册表
的值（`set_role` 后会 best-effort 重注册以保持一致）。

---

## 7. 表插件 vs 诊断 skill

| | **表插件**（路径 1） | **诊断 skill**（路径 2） |
|--|---------------------|-------------------------|
| 贡献 | dataclass 表 + 写行 | `SKILL.md` + 可选 `steps.yaml` |
| 产出 | `python.my_table` | 结论 + SQL 步骤 / Agent 指引 |
| 使用 | `SELECT …` | `probing skill run <id>` |
| 适用 | 新**指标/事件**要落库 | 新**排查配方** |

可选**路径 3**：NCCL profiler cdylib → `nccl.proxy_ops`（culprit/victim）。见
[扩展机制](../design/extensibility.zh.md)。

---

## 下一步

| 目标 | 文档 |
|------|------|
| SQL 模式 | [SQL 分析](sql-analytics.zh.md) |
| 表结构 | [SQL 表目录](../reference/sql-tables.zh.md) |
| 多机 | [分布式](../design/distributed.zh.md) |
| 写插件 | [扩展机制](../design/extensibility.zh.md) |
| CLI / API | [API 参考](../api-reference.zh.md) |

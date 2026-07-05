# SQL 表目录

本页列出 Probing 中可通过 `probing query` 或 `probing.query()` 查询的所有内置
SQL 表。这是一份参考手册——如果你需要查询模式和操作指南，从 [SQL 分析](../guide/sql-analytics.zh.md)
开始。

每张表由 mmap 环形缓冲（MEMT）承载或由扩展 crate 动态注册。表按 schema 前缀
组织，反映数据来源：`python.*` 是训练和 Python 运行时数据，`cpu.*` / `gpu.*`
是主机和设备采样，`cluster.*` 是节点注册信息，`nccl.*` 是 NCCL profiler 插件
输出，`global.<schema>.<table>` 是跨 rank 的联邦查询。

Schema 定义以 `skills/semantic/tables.yaml` 为权威来源（诊断 skill 与 Web Agent
使用该文件定义），本页与其保持同步。

在真实端点上查看当前可用表：

```bash
probing $ENDPOINT tables
probing $ENDPOINT tables --all
```

## Schema 前缀

| 前缀 | 数据来源 |
|------|----------|
| `python.*` | 训练和 Python 运行时（memtable） |
| `cpu.*`、`gpu.*`、`process.*` | 主机和设备采样（扩展 crate） |
| `cluster.*` | 集群节点注册表 |
| `nccl.*` | NCCL profiler 插件（可选，cdylib） |
| `global.<schema>.<table>` | 跨已注册节点联邦 fan-out |
| `information_schema.*` | 引擎元数据和配置 |

## 联邦查询

带 **`global_name`** 的表可用 `global.<路径>` 查询（如 `global.python.comm_collective`）。
Master 合并各节点结果并可能附加：

| 标签 | 说明 |
|------|------|
| `_host` | 来源主机名 |
| `_addr` | 来源 probing HTTP 地址 |
| `_rank` | `torch.distributed` rank（来自 `cluster.nodes`） |
| `_role` | 并行角色 key（如 `dp=2,pp=1,tp=0`） |

示例：

```sql
SELECT _role, _rank, avg(duration_ms) AS avg_ms
FROM global.python.comm_collective
GROUP BY _role, _rank;
```

---

## 训练与 tracing（`python.*`）

### `python.torch_trace` {#python-torch_trace}

PyTorch 模块级 forward/step 耗时与 GPU 显存快照。

**同义词：** torch trace、模块耗时

| 列 | 说明 |
|----|------|
| `local_step` | 本地训练步（每 rank） |
| `global_step` | 全局步（`step_snapshot`） |
| `rank` | `torch.distributed` rank |
| `world_size` | world size |
| `role` | 并行角色 key，如 `dp=2,pp=1,tp=0` |
| `seq` | 步内 hook 序号 |
| `module` | 模块全名 |
| `stage` | `pre forward`、`post forward`、`pre step`、`post step` |
| `duration` | hook 耗时（秒）；post 行有效 |
| `time_offset` | 相对 step 时间锚点（秒） |
| `allocated` | GPU 已分配显存（MB） |
| `allocated_delta` | 相对上一 hook 的 allocated 变化（MB） |
| `max_allocated` | 峰值 allocated（MB） |
| `max_allocated_delta` | 峰值 allocated 变化（MB） |
| `cached` | GPU 预留显存（MB） |
| `max_cached` | 峰值预留（MB） |

**说明：** 第一个完整 step 为 discovery（可能无行）。默认不采 backward hook。

---

### `python.comm_collective` {#python-comm_collective}

`torch.distributed` 集合通信（all_reduce、broadcast 等）。

**同义词：** collective、通信、all_reduce

| 列 | 说明 |
|----|------|
| `local_step` | 本 rank 本地步 |
| `global_step` | 全局训练步 |
| `rank` | `torch.distributed` rank |
| `world_size` | world size |
| `role` | 并行角色 key |
| `op` | 集合操作名 |
| `group_rank` | 进程组内 rank |
| `group_size` | 进程组大小 |
| `participate_ranks` | 参与 rank（序列化） |
| `tensor_shape` | 张量 shape |
| `tensor_dtype` | 张量 dtype |
| `bytes` | 通信字节数 |
| `duration_ms` | 墙钟时间（毫秒） |
| `async_op` | 1 表示异步 collective |

**Global：** `global.python.comm_collective`
**联邦列：** `_host`、`_addr`、`_rank`、`_role`

---

### `python.trace_event`

Span 起止与自定义事件（分布式 tracing）。

| 列 | 说明 |
|----|------|
| `record_type` | `span_start` \| `span_end` \| `event` |
| `trace_id` | 同一 trace 内共享 |
| `span_id` | Span 唯一 id |
| `name` | Span / 事件名 |
| `phase` | 训练阶段（`forward`、`backward`、`optimizer`）或空 |
| `time` | 时间戳（纳秒） |
| `attributes` | JSON 元数据（rank、local_step 等） |

在 `span_id` 上 join `span_start` / `span_end` 可得时长。见 [分布式](../design/distributed.zh.md)。

---

### `python.backtrace`

Python + native 混合栈（**瞬时**，非历史全量）。

| 列 | 说明 |
|----|------|
| `func` | 函数名 |
| `file` | 源文件 |
| `lineno` | 行号 |
| `depth` | 栈深度（0 为最内层） |
| `frame_type` | `python` \| `native` |

先 `probing backtrace`，再 `SELECT … FROM python.backtrace`。

---

### `python.variables`

启用变量追踪时的变量快照。

| 列 | 说明 |
|----|------|
| `micro_step` | 训练 micro-step |
| `func` | 函数名 |
| `name` | 变量名 |
| `value` | 字符串表示 |

---

## 系统指标

### `cpu.utilization`

主机 CPU 与 RSS 采样。

| 列 | 说明 |
|----|------|
| `ts` | 采样时间（微秒） |
| `scope` | `process` \| `thread` |
| `rss_kb` | 常驻内存（KB），仅 process |
| `cpu_total_pct` | CPU 利用率（%） |
| `comm` | 线程/进程名 |
| `wchan` | 内核等待通道（Linux） |

---

### `gpu.utilization`

GPU 显存与利用率采样。

| 列 | 说明 |
|----|------|
| `ts` | 采样时间 |
| `used_bytes` | 已用显存 |
| `total_bytes` | 总显存 |
| `mem_used_pct` | 显存使用率（%） |
| `gpu_util_pct` | GPU 算力利用率（不可用为 -1） |

---

### `process.kmsg`

Linux 内核环缓冲（dmesg）。**仅 Linux。**

| 列 | 说明 |
|----|------|
| `timestamp` | 事件时间 |
| `level` | 日志级别 |
| `message` | 内核消息 |

---

## 集群

### `cluster.nodes`

已注册的分布式训练节点。

| 列 | 说明 |
|----|------|
| `host` | 主机名 |
| `addr` | probing HTTP 地址 |
| `rank` | 全局 rank |
| `world_size` | world size |
| `local_rank` | 节点内 local rank |
| `role` | 并行角色 key（联邦 `_role` 来源） |
| `role_name` | Torchrun role 名（与 `role` 不同） |
| `status` | 节点状态 |
| `timestamp` | 最近更新时间（微秒） |

```bash
probing -t <master> cluster nodes
```

---

## NCCL profiler（可选）

见 [NCCL Profiler](../design/nccl-profiler.zh.md)。

### `nccl.proxy_ops`

Proxy-op 等待分解（culprit / victim）。

| 列 | 说明 |
|----|------|
| `send_gpu_wait_ns` | **Culprit** — 本地 GPU 未就绪 |
| `send_peer_wait_ns` | 等待接收端 clear-to-send credits（仅 v4 ABI；v3 为 0） |
| `recv_wait_ns` | **Victim** — 等待对端数据 |
| `send_wait_ns` | 发送侧网络等待 |
| `recv_flush_wait_ns` | Recv flush 等待 |
| … | 另有 `rank`、`coll_func`、`seq`、`trans_bytes` 等 |

**Global：** `global.nccl.proxy_ops`
**联邦列：** `_host`、`_addr`、`_rank`、`_role`

### `nccl.net_qp`

NetPlugin IB QP 完成耗时（可选）。

**Global：** `global.nccl.net_qp`

---

## 元数据

### `information_schema.df_settings`

运行时配置（`probing.*`）。

| 列 | 说明 |
|----|------|
| `name` | 配置键 |
| `value` | 配置值 |

---

## 自定义表

插件通过 `@table` dataclass 注册 `python.<name>`，schema 由作者定义。见
[扩展机制](../design/extensibility.zh.md)。

---

## 相关文档

- [核心概念](../guide/concepts.zh.md)
- [SQL 分析](../guide/sql-analytics.zh.md)
- [API 参考](../api-reference.zh.md)

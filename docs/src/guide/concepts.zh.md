# 核心概念

逐步构建 Probing 的心理模型——endpoint 是什么、数据如何流入表、step 坐标如何
组织状态、以及联邦查询如何跨节点工作。在深入 [SQL 分析](sql-analytics.zh.md)或
[分布式](../design/distributed.zh.md)之前请先阅读本文。

## Endpoint：如何访问 probing 服务器

每个启用 probing 的进程都运行一个嵌入式 HTTP 服务器。从外部与其通信需要一个
**endpoint**——可以是本地 PID 或 `host:port` 对。

```bash
# 本地进程——probing 从 PID 解析 Unix socket
probing -t 12345 query "SELECT 1"

# 远程进程——TCP 连接到已知地址
probing -t node-a:8080 query "SELECT 1"
```

CLI 从不直接与引擎交互。它通过 Unix socket（本地）或 TCP（远程）向目标进程
中嵌入的服务器发送 HTTP 请求。不存在 `probing.connect()` Python API——
远程访问始终通过 CLI 的 `-t` 参数。

在训练脚本内部（in-process 模式），则完全跳过 CLI，直接调用 `probing.query()`。
引擎已经在同一进程中运行。

启动时设置 `PROBING=1` 通过 `.pth` 钩子激活进程内服务器——无需 import、无需
修改代码。在 Linux 上，`probing inject` 也可以通过 ptrace 附着到已运行的进程。

概念上：

```
CLI  ──(HTTP over Unix socket/TCP)──▶  probing server（目标进程内）
                                          │
                                          ├── Engine（DataFusion）
                                          ├── Config
                                          └── Extensions（CPU、GPU、Python、NCCL...）
```

## 数据表：只追加、持续写入

Probing 将性能数据存储在 mmap 环形缓冲支持的**只追加 SQL 表**中。事件发生时
写入——模块 hook 触发、collective 完成、span 结束。不存在轮询，不按需快照。
数据已经在那里了。

最重要的表位于 `python` schema 下，因为这里是训练语义所在：

`python.torch_trace`
: 模块级 forward/backward/step hook 耗时与 GPU 显存快照。每次 hook 触发一行。
列包括 `local_step`、`module`、`stage`、`duration`、`allocated`。

`python.comm_collective`
: 每个 `torch.distributed` 集合调用（all_reduce、broadcast、all_gather 等）。
记录 `op`、`tensor_shape`、`bytes`、`duration_ms` 以及进程组上下文。

`python.trace_event`
: Span 起止事件和自定义 trace 事件。在 `span_id` 上 join `span_start` / `span_end`
计算耗时。Span 可以嵌套——forward pass span 包含多个 layer span。

`python.backtrace`
: 最新捕获的调用栈，混合 Python 和原生帧。**瞬时数据**，不是历史全量。
先用 `probing backtrace` 填充，再查询。用于 hang 诊断。

`python.variables`
: 变量快照（需显式启用变量追踪）。轻量级：值以字符串方式存储，不序列化。

除 `python.*` 之外，主机级数据在 `cpu.*` 和 `gpu.*`，集群元数据在 `cluster.*`，
NCCL profiler 输出在 `nccl.*`。完整的列定义见 [SQL 表目录](../reference/sql-tables.zh.md)。

自定义表遵循相同模型。用 `@table("my_metrics")` 定义 dataclass，在训练循环中
追加行，表即显示为 `python.my_metrics`——与内置表一同查询。见 [扩展机制](../design/extensibility.zh.md)。

## Step 坐标：训练分析的共享索引

训练分析中，关联不同表的数据需要一个共享的时间轴。Probing 使用 **step 坐标**
而非时间戳——它们具有确定性，且与训练语义天然对齐。

有三个层级：

**micro_step** —— 最细粒度计数器。每次调用 `probing.step()` 加一。
**local_step** —— optimizer step。`micro_step // micro_batches`。
**global_step** —— 集群范围的 step。rank 对齐时等同于 `local_step`。

```python
import probing

probing.step(micro_batches=10)   # 10 个 micro-batch = 1 个 optimizer step
probing.step()                   # micro_step += 1
probing.step(42)                 # 直接设置 micro_step

print(probing.step.micro_step)   # 原始计数器
print(probing.step.local_step)   # micro_step // 10
print(probing.step.global_step)  # 集群 step
```

所有训练相关表都携带 step 列：`python.torch_trace` 有 `local_step` 和 `global_step`；
`python.comm_collective` 同样具备，外加 `group_rank` 和 `group_size`。编写查询时，
用 `local_step` 或 `global_step` 过滤——不要用 `trainer.current_step`。

## Role：编码并行拓扑

分布式训练将每个 rank 置于并行拓扑中——tensor parallel、pipeline parallel、data
parallel、expert parallel 或它们的组合。Probing 将其编码为一个紧凑字符串，而非
每个维度一列。

格式为排序的 `name=value` 对：`dp=2,pp=1,tp=0`。空字符串表示 role 未设置。

可通过环境变量（Megatron 风格的 `*_PARALLEL_RANK` 或 `PROBING_ROLE_<NAME>=<int>`）
或 Python 设置：

```python
import probing
probing.set_role(dp=2, pp=1, tp=0)
# 或: probing.set_role("dp=2,pp=1,tp=0")

print(probing.current_role())   # "dp=2,pp=1,tp=0"
probing.clear_role()            # 恢复为环境变量默认值
```

`role` 被标记在所有 `python.torch_trace` 和 `python.comm_collective` 行上。
这让你可以 GROUP BY role 来对比，比如跨所有 data-parallel 副本比较 TP rank 0
和 TP rank 1。需区分 torchrun 的 `role_name` / `role_rank`（`cluster.nodes` 上
的字段）——这些是 launcher 字段；`role` 是用于分析的 key。

## 联邦查询：跨节点查询

当多个 rank 注册到集群时，`global.<schema>.<table>` 将查询 fan-out 到每个节点
并合并结果。查询在每个节点上独立执行；master 收集并拼接。

每个返回行附带四个联邦标签：

`_host`
: 生成行的 probing 节点主机名。

`_addr`
: 该节点的 `host:port` probing 地址。

`_rank`
: 来自集群节点注册的 `torch.distributed` rank。

`_role`
: 来自节点注册的并行 role key（与 `set_role` 保持同步）。

典型的联邦查询：

```sql
SELECT _role, _rank, op, AVG(duration_ms) AS avg_ms
FROM global.python.comm_collective
WHERE local_step > 100
GROUP BY _role, _rank, op
ORDER BY avg_ms DESC;
```

通过 torchrun（`setup_torchrun_cluster`）或 POST `/apis/nodes` 注册节点。
用 `probing -t <master> cluster nodes` 验证。详见 [分布式](../design/distributed.zh.md)。

## 表插件 vs 诊断 skill

Probing 有两条扩展路径，选择哪条取决于你的目的：

**表插件**添加新数据——用 `@table` 定义 dataclass，从代码追加行，数据以
`python.<name>` 出现。用于存储和查询新的指标或事件。

**诊断 skill** 添加新分析——`SKILL.md` 文件配合可选的 `steps.yaml`，描述基于
现有表的诊断工作流。Skill 运行 SQL 查询、应用解释规则、产出诊断结论。用于
沉淀排查方案。用 `probing skill run <id>` 运行。

NCCL profiler 是第三条路径——编译好的 cdylib，写入 `nccl.proxy_ops` 用于
culprit/victim 等待分解。它是 Rust 扩展，而非 Python @table。见 [NCCL Profiler](../design/nccl-profiler.zh.md)。

## 下一步

| 目标 | 文档 |
|------|------|
| 训练分析的 SQL 模式 | [SQL 分析](sql-analytics.zh.md) |
| 每张表的列定义 | [SQL 表目录](../reference/sql-tables.zh.md) |
| 多节点配置和 torchrun | [分布式](../design/distributed.zh.md) |
| 编写自定义表或 skill | [扩展机制](../design/extensibility.zh.md) |
| CLI 命令和 Python API | [API 参考](../api-reference.zh.md) |

# 架构概览

贡献者向设计文档。操作员见 **[用户指南](../guide/index.zh.md)**；契约见 **[参考手册](../reference/index.zh.md)**。

术语：**[核心模型](../guide/concepts.zh.md)**。

## 总体架构

![Probing 从使用入口到本地存储的整体架构](../assets/architecture/probing-top-down-overview.svg)

图中的两条方向需要分开理解：训练回调只向本机表追加数据；CLI、Web、Skill 和 MCP 的
读取请求则通过服务端进入查询引擎。采集器之间不直接调用，跨数据源关系在 SQL 层组合。

## 阅读顺序

1. **[模块化与边界](modularity.zh.md)** — 先建立模块归属和依赖方向。
2. **[启用、注入与运行时控制](activation-injection.zh.md)** → **[数据层](data-layer.zh.md)** —
   理解 Probing 如何进入进程，以及数据如何被持续保存。
3. **[性能分析与 Tracing](profiling.zh.md)** — 理解 TorchProbe、Span、训练阶段和堆栈采集。
4. **[分布式成员与控制面](distributed.zh.md)** → **[联邦查询](federation.zh.md)** —
   理解成员发现、分层 fan-out 与跨 rank SQL。
5. **[分布式 Profiler 查询与可视化](distributed-profiler.zh.md)** — 理解万 Rank Timeline
   数据模型、分层执行和跨 Rank 下钻。

表中“当前”表示描述现有实现；“草案/目标设计”表示尚未全部落地，不能当作已发布能力。

## 基础架构

| 文档 | 状态 | 说明 |
|------|------|------|
| [模块化与边界](modularity.zh.md) | 当前 | 分层、公开契约、归属边界 |
| [启用、注入与运行时控制](activation-injection.zh.md) | 当前 | `.pth`、ptrace、shellcode 跳板与服务就绪 |
| [数据层](data-layer.zh.md) | 当前 | MEMT/MEMC 热冷列存与 SQL 集成 |
| [扩展机制](extensibility.zh.md) | 当前 | `@table`、Rust collector、Skill 和公开服务契约 |

## 采集与 Profiling

| 文档 | 状态 | 说明 |
|------|------|------|
| [性能分析与 Tracing](profiling.zh.md) | 当前 | TorchProbe、Span/Phase、Python/Native 堆栈与系统采集 |
| [NCCL Profiler](nccl-profiler.zh.md) | 当前 | 插件 ABI、事件层次和等待分解 |
| [开销测量](overhead.zh.md) | 当前 | shadow step、统计口径、回归不变量和离线基准 |

## 分布式查询与分析

| 文档 | 状态 | 说明 |
|------|------|------|
| [分布式成员与控制面](distributed.zh.md) | 当前 | torchrun 注册、heartbeat、TTL 与成员元数据 |
| [联邦查询引擎](federation.zh.md) | 当前 | `global.*`、路径选择、分层 fan-out、标签和 partial 语义 |
| [分布式 Profiler 查询与可视化](distributed-profiler.zh.md) | 目标设计 | 万 Rank Timeline、分层查询、多分辨率视图和分布式火焰图 |

用户向工作流：**[用户指南](../guide/index.zh.md)** · 参考：**[SQL 表目录](../reference/sql-tables.zh.md)** · **[CLI 与 Python API](../api-reference.zh.md)**

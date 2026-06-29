# 架构概览

贡献者向设计文档。操作员见 **[用户指南](../guide/index.zh.md)**；契约见 **[参考手册](../reference/index.zh.md)**。

术语：**[核心模型](../guide/concepts.zh.md)**。

## 阅读顺序

1. **[模块化与边界](modularity.zh.md)** — 四层模型、crate 地图、依赖规则（从这里开始）
2. **[数据层](data-layer.zh.md)** — MEMT/MEMC、mmap、SQL 集成
3. **[分布式 → 概览](distributed.zh.md)** — 多节点心智模型，再读下方子页

## 平台核心

| 文档 | 说明 |
|------|------|
| [模块化与边界](modularity.zh.md) | L1–L4 分层、公开契约、归属边界 |
| [数据层](data-layer.zh.md) | 热/冷列存与 SQL 集成 |
| [扩展机制](extensibility.zh.md) | `@table` 插件、skills、NCCL profiler |
| [CLI 命令树](cli.zh.md) | 命令分组、target 规则、迁移方案（草案） |

## 采集与 Profiling

| 文档 | 说明 |
|------|------|
| [性能分析](profiling.zh.md) | Torch hook、采样、落表路径 |
| [NCCL Profiler](nccl-profiler.zh.md) | 插件 ABI、proxy 等待分解 |
| [调试引擎](debugging.zh.md) | eval / backtrace / REPL 实现 |
| [训练阶段](training-phase.zh.md) | 阶段转换与 span 模型 |

## 分布式

| 文档 | 说明 |
|------|------|
| [概览](distributed.zh.md) | 多节点拓扑、控制面、联邦入门 |
| [torchrun 集群心跳](torchrun-cluster.zh.md) | 分层注册、退避、环境变量 |
| [联邦查询引擎](federation.zh.md) | 跨 rank SQL 路径 A/B/C、标签、回归查询 |
| [分层集群查询](hierarchical-fanout.zh.md) | coordinator → local0 → leaf 聚合 |
| [基于 Pulsing 的集群](cluster-pulsing.zh.md) | 可选 Pulsing 成员发现 |

## 旧版

| 文档 | 说明 |
|------|------|
| [系统架构（旧版）](architecture.zh.md) | 两层概览 — 已由 [模块化](modularity.zh.md) 取代 |

用户向工作流：**[用户指南](../guide/index.zh.md)** · 参考：**[SQL 表目录](../reference/sql-tables.zh.md)** · **[CLI 与 Python API](../api-reference.zh.md)**

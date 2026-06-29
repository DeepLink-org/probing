# 用户指南

面向操作员的命令与 SQL 用法。列定义、HTTP、环境变量见 **[参考手册](../reference/index.zh.md)**；
实现见 **[架构](../design/index.zh.md)**。

术语：**[核心模型](concepts.zh.md)**。

## 接口

| 接口 | 输入 | 输出 / 副作用 |
|------|------|----------------|
| `probing query` | `probe.*` / `global.*` 上的 SQL | `DataFrame` JSON |
| `probing eval` | Python 源码 | 目标解释器 stdout / 异常 |
| `probing backtrace` | — | 写入 `python.backtrace` |
| `probing skill run` | skill id + 参数 | 执行 `skills/*/steps.yaml` |
| `probing cluster nodes` | — | `cluster.nodes` 注册视图 |
| `probing cluster query` | SQL + `cluster=true` | 按 [联邦引擎](../design/federation.zh.md) fan-out |

内置表：`python.torch_trace`、`python.comm_collective`、`gpu.utilization`、`nccl.proxy_ops` 等 — 见 **[SQL 表目录](../reference/sql-tables.zh.md)**。

## 页面顺序

1. [SQL 分析](sql-analytics.zh.md) — `global.*`、联邦标签、JOIN 规则
2. [诊断 Skill](skills.zh.md) — `steps.yaml` 运行器
3. [内存分析](memory-analysis.zh.md) — `python.torch_trace`、GPU 表
4. [现场调试](debugging.zh.md) — backtrace、eval、REPL
5. [常见问题](troubleshooting.zh.md) — HTTP、inject、空表

入门路径：安装指南 → 快速开始 → 核心模型（导航 **入门**）。

## 架构索引

- [模块化与边界](../design/modularity.zh.md) — L1–L4
- [分布式概览](../design/distributed.zh.md) — 成员、fan-out
- [扩展机制](../design/extensibility.zh.md) — `@table`、skills

CLI 完整列表：**[CLI 与 Python API](../api-reference.zh.md)**。

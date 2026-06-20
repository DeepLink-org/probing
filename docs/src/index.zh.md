---
template: home.html
title: Probing - 分布式 AI 动态性能分析器
description: 面向分布式 AI 的零侵入分析器 — SQL 表、现场内省、集群联邦与诊断 skill。
hide: toc
---

# Probing

**Probing** 面向分布式 AI 训练：持续写入 SQL 表、现场附着、联邦 `global.*` 查询与内置诊断 skill。

## 能力概览

- **持续采集** — `torch_trace`、`comm_collective`、NCCL proxy、自定义 `@table`
- **现场内省** — 对运行中进程 `eval`、`backtrace`、REPL
- **SQL 分析** — 单节点与 `global.*` 联邦（`_rank` / `_role` 标签）
- **诊断 skill** — `health_overview`、`slow_rank`、`nccl_culprit_victim` 等
- **集群** — `cluster nodes`、`cluster query`、Web UI Agent
- **零侵入** — 启动时 `PROBING=1` 或 Linux `inject`

## 快速开始

```bash
pip install probing

# 训练推荐
PROBING=1 PROBING_TORCH_PROFILING=on python train.py

# 或 Linux 附着
probing -t <pid> inject
probing -t <pid> query "SELECT * FROM python.torch_trace LIMIT 10"
probing -t <pid> skill run health_overview
```

## 文档导航

- [安装指南](installation.zh.md) · [快速开始](quickstart.zh.md) · [核心概念](guide/concepts.zh.md)
- [SQL 表目录](reference/sql-tables.zh.md) · [API 参考](api-reference.zh.md) · [贡献指南](contributing.zh.md)

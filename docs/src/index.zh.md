---
template: home.html
title: Probing - 分布式 AI 动态性能分析器
description: 附着到运行中的 Python 训练进程，用 SQL 查询性能数据，跨集群节点诊断分布式问题。
hide: toc
---

# Probing

无需修改代码即可检查和调试分布式 AI 训练任务。附着到运行中的 Python 进程，用标准 SQL
查询性能数据，运行诊断工作流定位慢 rank、NCCL 瓶颈或内存泄漏。

## 30 秒上手

```bash
pip install probing

# 启动带 probing 的训练
PROBING=1 python train.py &

# 附着并检查
probing -t $(pgrep -f train.py) backtrace
probing -t $(pgrep -f train.py) query "
  SELECT module, stage, AVG(duration) as sec
  FROM python.torch_trace
  GROUP BY module, stage
  ORDER BY sec DESC LIMIT 5
"
```

第一条命令启动一个已激活 probing 的训练任务。第二条捕获主线程上所有 Python 和原生帧。
第三条用一条 SQL 查出最慢的五个模块-阶段组合——无需加日志、无需插桩、无需重启。

## 能做什么

**调试卡住或变慢的训练任务。**
附着到卡住的进程，抓取调用栈，按 step 查看 GPU 内存变化，精确定位是哪个模块或集合通信
调用在阻塞。不需要复现问题。

**诊断分布式集合通信性能。**
NCCL profiler 插件将 proxy-op 等待时间分解为 send/recv 延迟，能清晰判断谁在等谁——
调试 all-reduce 尾部延迟的关键工具。

**编写自定义性能表。**
用 `@table("my_metrics")` 定义 dataclass，从训练循环中追加行，和内置表一起查询。
数据存在 `python.my_metrics`，同一命名空间，同一 SQL 接口。

**跨集群节点查询。**
在任何表前加 `global.` 前缀即可 fan-out 到已注册节点。每行附带 `_rank`、`_role`、
`_host` 标签，结果可直接跨集群对比。

## 工作原理

Probing 以 Python 包形式发布，内置编译好的 Rust 核心（`probing._core`）。
当你运行 `PROBING=1 python train.py` 时，一个 `.pth` 钩子启动进程内 HTTP 服务器，
为 SQL 引擎注册数据源。扩展模块——CPU 采样、GPU 显存、NCCL proxy ops、Python 调用栈
追踪——将行数据推送到 mmap 环形缓冲区支持的只追加列式表中。CLI 通过 Unix socket
（本地）或 TCP（远程）与嵌入式服务器通信。

作为用户，你不需要了解这些细节。`pip install probing` 就够了。

## 从这里开始

**我想调试训练问题。**
阅读[快速开始](quickstart.zh.md)，然后试试 `probing backtrace` 和 `probing query`。

**我想理解架构。**
阅读[核心概念](guide/concepts.zh.md)和[模块化与边界](design/modularity.zh.md)。

**我在搭建多节点集群。**
阅读[分布式](design/distributed.zh.md)和 [SQL 表目录](reference/sql-tables.zh.md)。

**我想写一个自定义诊断 skill。**
阅读[扩展性](design/extensibility.zh.md)，浏览 `skills/` 目录下的示例。

**我想贡献代码。**
阅读[贡献指南](contributing.zh.md)，执行 `make develop`，挑一个 issue 开始。

## 文档导航

| 文档 | 内容 |
|------|------|
| [安装指南](installation.zh.md) | `pip install`、`PROBING=1`、平台支持 |
| [快速开始](quickstart.zh.md) | 5 分钟上手，真实调试场景 |
| [核心概念](guide/concepts.zh.md) | Probing 工作原理——心理模型、数据流、step 坐标、联邦查询 |
| [SQL 表目录](reference/sql-tables.zh.md) | 每张内置表的列定义 |
| [API 参考](api-reference.zh.md) | CLI 命令和 Python API |
| [环境变量](reference/env-vars.md) | 全部 30+ 个 `PROBING_*` 环境变量参考 |
| [Skill 格式规范](reference/skill-format.md) | `steps.yaml` 和 `SKILL.md` 格式规范 |
| [SQL 分析](guide/sql-analytics.zh.md) | 查询模式、JOIN 示例、时序分析 |
| [诊断 Skill](guide/skills.zh.md) | 运行和编写诊断工作流 |
| [扩展机制](design/extensibility.zh.md) | 数据表插件、诊断 skill、NCCL profiler |
| [分布式](design/distributed.zh.md) | 多节点联邦、torchrun 集成 |
| [NCCL Profiler](design/nccl-profiler.zh.md) | NCCL 插件、proxy-op 等待分解 |
| [贡献指南](contributing.zh.md) | 开发环境搭建、PR 流程 |

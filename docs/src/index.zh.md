---
template: home.html
title: Probing - 分布式 AI 动态性能分析器
description: 面向 Python 训练负载的进程内 profiler，提供 SQL 查询接口。
hide: toc
---

# Probing

面向 Python 训练任务的进程内 profiler。Rust 核心（`probing._core`）嵌入 HTTP 服务器与
DataFusion SQL 引擎；采集器向 mmap memtable 追加行；CLI 与 Web 经 Unix socket 或 TCP 发查询。

## 组件

| 组件 | 位置 | 作用 |
|------|------|------|
| **Probe** | 目标 Python 进程 | HTTP 服务、引擎、扩展注册 |
| **Engine** | `probing/core` | DataFusion catalog、联邦改写、`async_query` |
| **Memtable** | `probing/memtable` | MEMT 环形缓冲、可选 MEMC 冷段 |
| **Collectors** | `probing/extensions/*`、`python/probing/` | CPU、GPU、NCCL、torch hook → SQL 表 |
| **CLI / Web** | `probing/cli`、`web/` | HTTP 客户端；运行时不对链 engine |

启用方式：进程启动时 `PROBING=1`（`.pth` hook），或 Linux 上 `probing -t <pid> inject`。

## 最小示例

```bash
pip install probing
PROBING=1 python train.py &

probing -t $(pgrep -f train.py) query "
  SELECT module, stage, avg(duration) AS sec
  FROM python.torch_trace
  GROUP BY module, stage
  ORDER BY sec DESC LIMIT 5
"
```

`backtrace` 写入 `python.backtrace`；`query` 读取 `probe` catalog 下已注册表；配置集群成员后
`global.*` 跨 rank fan-out（见 [分布式概览](design/distributed.zh.md)）。

## 文档

| 章节 | 内容 |
|------|------|
| [安装指南](installation.zh.md) | PyPI、wheel、平台、`PROBING` 模式 |
| [快速开始](quickstart.zh.md) | attach、inject、首条查询 |
| [核心模型](guide/concepts.zh.md) | 进程内 vs inject、catalog、step 坐标 |
| [用户指南](guide/index.zh.md) | SQL、skill、调试命令 |
| [架构](design/index.zh.md) | 分层、存储、联邦、采集器 |
| [参考手册](reference/index.zh.md) | SQL 表、CLI/API、环境变量 |

贡献者：[贡献指南](contributing.zh.md) · 文档风格：[writing.md](writing.md)

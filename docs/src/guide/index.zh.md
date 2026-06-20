# 用户指南

使用 Probing 分析与调试 AI 训练的操作指南。

## Probing 做什么

| 层次 | 产出 | 方式 |
|------|------|------|
| **持续采集** | `python.torch_trace`、`python.comm_collective`、span、插件表 | 钩子随训练追加行 |
| **现场内省** | 查看 live 对象、抓栈 | CLI `eval`、`backtrace`（或进程内 API） |
| **SQL 分析** | 临时与联邦查询 | `query`、`global.*`、`cluster query` |
| **诊断 skill** | 多步调查剧本 | `probing skill run <id>` |

术语：**[核心概念](concepts.zh.md)**。列定义：**[SQL 表目录](../reference/sql-tables.zh.md)**。

## 阅读顺序

新用户请走导航 **入门**：安装指南 → 快速开始 → 核心概念。

然后读本指南：

1. **[SQL 分析](sql-analytics.zh.md)** — 查询、`global.*`、`_role`
2. **[诊断 Skill](skills.zh.md)** — `health_overview`、`slow_rank` 等
3. **[内存分析](memory-analysis.zh.md)** — 泄漏与 GPU 压力
4. **[调试指南](debugging.zh.md)** — backtrace / eval 工作流
5. **[常见问题](troubleshooting.zh.md)** — 典型故障

## 主要 CLI 命令

| 命令 | 作用 |
|------|------|
| `query` | 读取采集表 |
| `eval` | 在目标进程执行 Python |
| `backtrace` | 抓栈 → `python.backtrace` |

完整 CLI：**[API 参考](../api-reference.zh.md)**。

## 设计文档

- **[系统架构](../design/architecture.zh.md)** — 探针、引擎、扩展
- **[分布式](../design/distributed.zh.md)** — 集群与联邦
- **[扩展机制](../design/extensibility.zh.md)** — `@table` 插件

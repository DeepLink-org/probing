# 诊断 Skill

**Skill** 是针对常见训练问题的多步 SQL 剧本，带版本与参数。通过 `probing.skills`
entry point 打进 wheel（`python/probing/bundled_skills/`），由共享 Rust runner
（`probing-skills`：CLI、Web Agent、MCP）执行，并在运行时从内置树、仓库 checkout、
已安装的 `probing-<vendor>` 包中发现。

## 何时用 skill、何时写 SQL

| 方式 | 适用 |
|------|------|
| **`probing skill run <id>`** | 已知场景（卡住、慢 rank、内存泄漏）— 固定步骤与阈值 |
| **`probing query "…"`** | 临时探索、自定义看板 |
| **`cluster query` + `global.*`** | 跨 rank 对比（联邦标签） |

Skill 读取 **[SQL 表目录](../reference/sql-tables.zh.md)** 中的同一批表。参数与过滤请用
`step_snapshot()` 坐标及 `_role` / `_rank` 标签，**不要**依赖各框架的 `trainer.current_step`。

## 快速上手

```bash
export ENDPOINT=rank0:8080

probing $ENDPOINT skill list
probing $ENDPOINT skill run health_overview
probing $ENDPOINT skill run slow_rank --global
probing $ENDPOINT skill run nccl_culprit_victim --global
```

覆盖参数：

```bash
probing $ENDPOINT skill run module_bottleneck -p window_steps=50 -p top_n=15
```

## 内置 skill（0.2.x）

| ID | 类别 | 用途 |
|----|------|------|
| `health_overview` | 分诊 | 首次查看：利用率 + 表数据新鲜度 |
| `training_hang` | 可靠性 | 停滞、空闲线程、step 不前进 |
| `slow_rank` | 分布式 | `global.*` 找 straggler |
| `nccl_culprit_victim` | 分布式 | 集合通信等待不均衡 |
| `comm_bottleneck` | 分布式 | 通信 vs 计算占比 |
| `module_bottleneck` | 性能 | `torch_trace` 热点模块 |
| `gpu_pressure` | 内存 | VRAM 压力模式 |
| `memory_leak` | 内存 | 跨 step 分配增长 |

以本机 `probing skill list` 输出为准。

## 安装到编码 Agent

```bash
probing skill install --user
probing skill update --user
```

源码在仓库 `skills/`；打 wheel 前校验：

```bash
python -m probing.skills validate
make wheel
```

## 编写

```
skills/<id>/
  SKILL.md      # 意图、适用场景
  steps.yaml    # 有序 SQL 步骤与参数
```

Agent 路由元数据（`intents.yaml`、`pages.yaml`）位于 `skills/semantic/`。
表/列描述以采集器与 `@table` 代码注册为 SSOT；`probing/core/resources/tables.yaml` 为 L1 agent overlay
（synonyms、notes、global_name），在引擎启动时合并进 `probe.probing.table_docs` / `column_docs`。见
**[贡献指南](../contributing.zh.md)**。

## 相关

- **[SQL 分析](sql-analytics.zh.md)** — `global.*`、`_role` GROUP BY
- **[核心概念](concepts.zh.md)** — 联邦标签、step 坐标
- **[API 参考](../api-reference.zh.md)** — `skill` / `cluster` CLI

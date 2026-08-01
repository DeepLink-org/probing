# 文档维护指南

Probing 文档采用基础设施项目的写法：**契约优先、机制优先**。说明系统是什么、如何运行、
默认值和失败方式，避免产品宣传和无法验证的结果承诺。

## 推荐与避免

| 推荐 | 避免 |
|------|------|
| 结构、依赖、数据流和 invariant | 使命、愿景和营销性标题 |
| API、默认值、资源上限和失败语义 | “一定能找到根因”等结果承诺 |
| 中性标题与可复制命令 | 用故事替代契约 |
| 用表格表达 schema、变量和路径条件 | 在多个页面重复同一份定义 |

## 文档角色

| 区域 | 主要内容 |
|------|----------|
| **Getting Started** | 建立第一条连接，并提供可验证的完成条件 |
| **Guide** | 完成任务所需的命令、SQL、结果解释和下一步 |
| **Operations** | 信任边界、健康检查、资源限制、失败处理和 runbook |
| **Architecture** | 模块归属、协议、算法、invariant、已知限制和测试 |
| **Reference** | schema、flag、环境变量、HTTP DTO 等精确契约 |
| **Examples** | 可复现的输入、命令、预期证据和清理步骤 |

每个页面只属于一个主要区域。Guide 不重复实现设计，Reference 不承载教程；用链接指向
对应 SSOT。

## SSOT

| 内容 | 权威来源 |
|------|----------|
| HTTP endpoint 与 client 调用 | `tests/regression/spec/api_spec.json`；清单为 `probing/server/API.md` |
| 表与列语义 | Collector schema/table docs；发布索引为 `reference/sql-tables.md` |
| CLI 语法 | Clap 定义；`api-reference.md` 提供摘要 |
| 环境变量默认值 | 读取该变量的所属模块；`reference/env-vars.md` 提供摘要 |
| 架构边界 | `design/modularity.md` |
| 诊断工作流 | `skills/<id>/SKILL.md` + `steps.yaml` |

摘要可以指向 SSOT，但不能静默重新定义契约。

## 中英文

- 英文和中文页面保持相同的章节结构与契约表。
- 新导航项必须同时提供 `.md` 与 `.zh.md`，或提供明确链接英文版的 fallback 页面。
- 默认值、保证和安全边界变化时，同一变更内更新两种语言。
- 翻译机制和契约，不翻译口号。

## 交叉链接

- Architecture 将用法交给 Guide；Guide 将精确定义交给 Reference。
- Operations 负责部署选择、权限和故障处理。
- 一个主题只保留一个 SSOT，例如模块边界归 `modularity.md`，联邦语义归
  `federation.md`。

## 页面质量检查

- 首段说明范围并链接前置材料。
- 命令可复制，统一使用 `<pid>` 等 placeholder。
- 明确默认值、平台限制、失败/部分结果和安全影响。
- 只描述已实现行为；草案和提案必须标注状态。
- 用户可见变更同步更新 Guide/Operations 与对应 Reference。
- 英文与中文结构对齐。
- 相对链接在 `docs/src` 下可解析。
- 合并前运行 `make docs`，确保 MkDocs strict build 通过。

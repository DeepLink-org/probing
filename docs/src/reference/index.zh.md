# 参考手册

SQL 表结构、命令、wire protocol 与运行时配置的权威查阅入口。

| 页面 | 内容 |
|------|------|
| **[SQL 表目录](sql-tables.zh.md)** | `python.*`、`cluster.*` 物理列与联邦标签（与 `tables.yaml` 同步） |
| **[CLI 与 Python API](../api-reference.zh.md)** | CLI、进程内 Python API、配置、[未实现 API](../api-reference.zh.md#unimplemented-apis) |
| **[HTTP 与 MCP API](http-api.zh.md)** | Wire surface、认证边界、endpoint 契约与机器可读契约入口 |
| **[环境变量](env-vars.zh.md)** | 面向使用者的 `PROBING_*` 配置；中文覆盖部署/安全关键项，其余链接英文完整表 |
| **[Skill 格式规范](skill-format.md)** | 面向诊断 skill 作者的 `steps.yaml` 和 `SKILL.md` 格式规范 |
| **[版本兼容性](../versions.zh.md)** | 版本兼容与升级说明 |

## 契约归属

| 契约 | SSOT | 校验方式 |
|------|------|----------|
| HTTP route 与 client 调用 | `tests/regression/spec/api_spec.json` | `pytest tests/regression/spec/ -q` |
| SQL 表语义 | Collector schema 与 table docs | SQL 表参考 / regression tests |
| 环境变量 | 所属模块中的读取实现 | 文档 review 与严格站点构建 |
| Skills | `skills/<id>/SKILL.md` + `steps.yaml` | `python -m probing.skills validate` |

叙事性指南见 **[用户指南](../guide/index.zh.md)**；部署决策见
**[运行与运维](../operations/index.zh.md)**。

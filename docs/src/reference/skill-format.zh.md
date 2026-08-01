# Skill 格式规范

本页是中文导航摘要。字段级权威规范当前维护在完整的
[英文 Skill Format](skill-format.md)；修改 schema 时应以英文页、`probing-skills`
实现和校验器为准。

## 包结构

每个诊断 skill 位于 `skills/<skill_id>/`：

| 文件 | 作用 |
|------|------|
| `SKILL.md` | Agent 路由、使用条件和结果解释 |
| `steps.yaml` | 可执行步骤、参数、前置表、interpretation rule 与 next step |

总目录为 `skills/catalog.yaml`。Skill 通过 SQL 和已记录的 HTTP endpoint 访问系统，
不得导入 Rust/Python engine 内部实现。

## 最小维护流程

1. 在 `skills/<id>/` 修改 `SKILL.md` 与 `steps.yaml`。
2. 若增加表或 HTTP 依赖，先扩展对应公开契约。
3. 运行：

```bash
python -m probing.skills validate
pytest tests/regression/skills/ -q
```

字段、模板变量、step 类型和 interpretation grammar 见
[英文完整规范](skill-format.md)。

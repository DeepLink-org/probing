# 扩展机制

本文只定义 Probing 的公开扩展路径和它们之间的边界。`@table` 的完整 API 见
[CLI 与 Python API](../api-reference.zh.md#table-dataclass-plugins)，Skill 字段见
[Skill 格式规范](../reference/skill-format.md)，NCCL 表和部署见
[NCCL Profiler](nccl-profiler.zh.md)。这些细节不在本页重复维护。

## 1. 扩展模型

| 想扩展什么 | 公开机制 | 产生的契约 |
|------------|----------|------------|
| 新的训练/框架数据 | Python `@table` 插件 | `python.<table>` |
| 新的诊断方法 | `SKILL.md` + `steps.yaml` | 可复现 SQL 工作流 |
| REPL 快捷操作 | `probing.magics` entry point | IPython Magic |
| 厂商能力包 | `probing-<vendor>` wheel | skills、magics、可选表插件 |
| NCCL 内部事件 | NCCL Profiler C ABI cdylib | `nccl.*` 表 |

![新增事实进入采集层，新的分析与交互复用现有表契约](../assets/architecture/probing-feature-placement.svg)

核心原则是：数据扩展只增加表，分析扩展只查询公开表。不要为一个 skill 在 server 中加特例，
也不要让两个采集器直接调用彼此。

## 2. 表插件 {#path-1-table-plugin-dataclass--table}

`@table` 把 dataclass 固定为一个 append-only schema：

```python
from dataclasses import dataclass
from probing import table

@table
@dataclass
class StepStats:
    local_step: int
    global_step: int
    loss: float

def init():
    StepStats.init_table()
```

训练路径调用 `StepStats(...).save()`；查询侧读取 `python.step_stats`，多 rank 时读取
`global.python.step_stats`。插件可以由训练脚本直接 import，也可以通过
`probing -t <pid> config python.enabled=<module>` 管理生命周期。

边界要求：

- 字段首次创建后类型固定；破坏 schema 时使用新表名或显式重建。
- 行应是标量和小结构，不保存模型权重或大 payload。
- writer 失败应被隔离并记录日志，不能拖垮训练。
- step/rank/role 使用 Probing 的公共坐标，跨数据源关系通过 SQL JOIN 表达。
- `@table` 是追加事实，不是“每次 SQL 时扫描进程对象”的 pull API。

方法、命名、容量与启停参数见 [API 参考](../api-reference.zh.md#table-dataclass-plugins)和
[环境变量](../reference/env-vars.zh.md)。

## 3. 诊断 Skill {#path-2-diagnostic-skill}

Skill 不采集数据，它把“应该查什么、如何解释、下一步做什么”封装为版本化工作流：

```text
python/probing/bundled_skills/<id>/
  ├─ SKILL.md       路由、适用场景与解释
  └─ steps.yaml     参数化 SQL、空结果语义与确定性规则
```

![CLI、MCP 和 Web 复用 probing-skills 运行时](../assets/architecture/probing-skill-multiclient-runtime.svg)

| 阶段 | 唯一责任方 |
|------|------------|
| 内容 SSOT | `python/probing/bundled_skills/`；根 `skills/` 是符号链接别名 |
| 发现 | Python entry point / skills HTTP API |
| 加载、参数校验、执行、解释 | Rust `probing-skills` |
| 交互适配 | CLI、Web WASM、MCP 各自负责传输和展示 |

因此 Python 的 skills 工具只做发现/计划，不形成另一套 runner；CLI、Web 和 MCP 也不能各自
复制 YAML 解释逻辑。完整字段、示例和校验命令统一见
[Skill 格式规范](../reference/skill-format.md)与[诊断 Skill 指南](../guide/skills.zh.md)。

## 4. REPL 与厂商扩展包 {#path-3-repl-magic}

REPL Magic 通过 `probing.magics` 注册 IPython `Magics` 子类。第三方通常不单独发布 magic，
而是与 skill 和可选表插件一起放进厂商 wheel。

### 厂商包约定 {#path-4-vendor-extension-package-probing-vendor}

| 层级 | 约定 | 示例 |
|------|------|------|
| wheel | `probing-<vendor>` | `probing-nvidia` |
| import package | `probing_<vendor>` | `probing_nvidia` |
| skill / magic id | 带 vendor 前缀 | `nvidia_nccl_triage` |

```toml
[project]
name = "probing-nvidia"
dependencies = ["probing"]

[project.entry-points."probing.skills"]
nvidia = "probing_nvidia:skill_root"

[project.entry-points."probing.magics"]
nvidia = "probing_nvidia.magics:NvidiaMagic"
```

entry point 是发现契约；`package-data` 只负责把文件打进 wheel，不能替代注册。开发模板见
`examples/probing-acme/`。厂商包中的 `@table` 模块仍由 `python.enabled` 显式启用，不因安装
wheel 就自动进入训练热路径。

## 5. NCCL Profiler 特例 {#path-5-nccl-profiler-plugin}

NCCL Profiler 由 NCCL runtime 通过 C ABI 加载，不是 Python 动态插件。它写 `nccl.*` mmap
表，再由普通 SQL、Skill 和联邦查询消费。ABI 版本、事件 mask、表 schema、mock 和真机验收
只在 [NCCL Profiler](nccl-profiler.zh.md) 维护，本页不复制。

这个特例仍遵守相同上层边界：插件只产出数据，不调用 Skill、Web 或其他 collector。

## 6. 哪些不是公开扩展 API

| 机制 | 定位 |
|------|------|
| Rust `ProbeExtension` / `ProbeDataSource` | 内置模块契约，需要编译进 Probing |
| `@ext_handler` / `/apis/pythonext/*` | 核心 HTTP 实现，不是第三方稳定接口 |
| `add_module_callback` import hook | 官方框架集成内部能力 |
| `probing-*` 外部 CLI 二进制 | 独立工具发现，不等同于数据插件 |

第三方需要新的控制能力时，应先扩展公开表、Skill 或文档化 HTTP/proto 契约，而不是依赖内部
模块。内置 crate 的依赖和 ownership 见[模块化与边界](modularity.zh.md)。

## 7. 验收检查

1. 新事实是否通过表暴露，而不是 callback 链？
2. 新分析是否只使用 SQL/公开 HTTP？
3. 写入失败是否与训练主路径隔离？
4. 多 rank 是否使用固定的 `_rank`、`_role` 等联邦标签？
5. Skill 是否通过 `python -m probing.skills validate`？
6. 用户可见 schema、配置或接口是否同步参考手册和契约测试？

相关文档：[数据层](data-layer.zh.md) · [联邦查询](federation.zh.md) ·
[SQL 表目录](../reference/sql-tables.zh.md) · [核心模型](../guide/concepts.zh.md)

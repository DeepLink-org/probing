# Probing 实现质量与死代码审计

**日期：** 2026-07-11
**范围：** 全仓库 ~622 个 Rust/Python 源文件（不含 `target/`、`.venv/`）；Web WASM 与 bundled assets 部分抽查。
**方法：** 静态分析（`rg`/`allow(dead_code)`/import 图）、`modularity.md` §8 对照、子模块抽样读码确认。
**非目标：** 逐行人工 review 全部 17 万行 Rust；运行时覆盖率分析。

---

## 1. 执行摘要

| 维度 | 评分 (10) | 说明 |
|------|-----------|------|
| **架构分层** | 8.5 | L1–L4 边界清晰，`ProbeDataSource` / Skill / MCP 契约明确 |
| **错误处理** | 7.5 | `EngineError` 方向正确；federation / PyO3 仍有散落 `map_err` |
| **测试** | 7.0 | regression + unit 分层好；契约镜像 SQL 略重复 |
| **代码卫生** | 6.0 | 重构遗留：`tracing.rs`、Web dead 函数、9 份旧 JS bundle |
| **重复度** | 6.5 | Skill root 双实现、DataFrame 解析三份、NCCL mock schema 平行维护 |
| **综合** | **7.2** | 工程深度足够；**~1,200–2,000 行**可安全删除或合并（见 §6） |

**结论：** 实现质量整体高于典型 early-beta OSS，但存在一批**重构后未清理**的模块。最大单块债务是 `probing/core/src/tracing.rs`（874 行，完全未接入生产路径）。

---

## 2. 实现质量点评

### 2.1 做得好的地方

1. **模块化 SSOT 已基本落地**
   - Skills 执行：`probing/crates/skills`（CLI / MCP / Web 共用 `SkillBackend` trait）
   - Skills 内容：`skills/` → wheel 复制到 `bundled_skills/`
   - 文档：`docs/src/design/modularity.md` 自曝 technical debt，可追踪

2. **Collector 边界干净**
   - L2 扩展不写 SQL、不互调；跨信号分析走 JOIN / Skill steps
   - NCCL / Torch / GPU 三套 collective 数据源在 `nccl-profiler.md` 有语义区分

3. **API 契约可回归**
   - `tests/regression/spec/api_spec.json` + `test_api_spec.py`
   - Federation rewrite 有 legacy fallback 单测

4. **Agent 路径设计一致**
   - MCP read-only 默认、`PROBING_MCP_ALLOW_WRITE` 显式开启
   - Skill 仅 YAML + SQL，无 Rust/Python 业务逻辑散落

### 2.2 需要改进的地方

1. **「未接线」代码仍留在 core**
   - `tracing.rs` 注释写明 *Not wired into production paths yet*，却占 L1 crate

2. **Web 层遗留 Agent 辅助函数**
   - `web/src/agent/cluster.rs` 四个 `#[allow(dead_code)]` 函数；runner 已走 `WebBackend`

3. **Discovery / loader 双轨**
   - Rust `discovery.rs` 与 Python `extensions/skills.py` 各维护一套 skill root 解析

4. **Bundled 前端 artifact 未清理**
   - 10 个 `web-dxh*.js`，`index.html` 仅引用 1 个

5. **工具链小坑**
   - `web/dist` → `python/probing/bundled_web/public`  symlink 在部分 `rg` 扫描下报 ENOENT，影响全库 grep

---

## 3. 死代码与重构遗留（已核实）

按 **可删除风险** 排序。✅ = 建议删除；⚠️ = 保留但需标注；❌ = 误报（有用途）。

### 3.1 高优先级 — 建议删除

| ID | 路径 | 行数(约) | 证据 | 操作 |
|----|------|----------|------|------|
| D1 | `probing/core/src/tracing.rs` | 874 | L1–2 注释未接入；`lib.rs` 仅 `mod tracing` 私有模块，全库无 `tracing::` 引用 | 删除或 `feature = "internal-tracing"` + 移入 `tests/` |
| D2 | `python/probing/discovery.py` | 31 | 纯 re-export shim；`extensions/README.md:37` 标 Deprecated；`python/` 内零 `from probing.discovery` | 删除（或再保留 1 个 minor release） |
| D3 | `uv.lock.2` | 1307 | 非主 `uv.lock` 的备份 lockfile | 删除 |
| D4 | `python/probing/bundled_web/public/assets/web-dxh*.js`（9 个） | — | `index.html:14,17` 仅引用 `web-dxhe761fc9584f257f1.js` | 删除旧 bundle；`make frontend` 后加清理脚本 |
| D5 | `web/src/agent/cluster.rs` | L83–135 | `sql_needs_cluster_fanout` / `format_cluster_meta` / `default_use_global` / `execute_sql_for_agent` 均 `dead_code`，无调用 | 删除未用函数；保留 `cluster_context_for_llm` |
| D6 | `web/src/agent/routing.rs` | L102+ | `route_for_page` `#[allow(dead_code)]`，无调用 | 删除或接入 deep-link |
| D7 | `web/src/state/investigation.rs` | L310+ | `set_process_context` `dead_code` | 删除或接入 Dashboard |
| D8 | `web/src/hooks/mod.rs` | L107+ | `use_poll_tick` 薄包装；页面均用 `use_poll_tick_gated` | 删除 |
| D9 | `web/src/api/traces.rs` | L312, L391 | `get_ray_timeline` + `RayTimelineEntry` dead；UI 用 `get_ray_timeline_chrome_format` | 删除 |
| D10 | `web/src/agent/skills_backend.rs` | L87–93 | `_query_envelope_unused()` 仅为强制类型引用 | 删除 |
| D11 | `probing/core/src/core/engine.rs` | L151–158 | `#[deprecated] pub fn query`；生产路径均 `async_query` | 删除 deprecated API |
| D12 | `probing/core/src/core/mod.rs` | L59 附近 | 注释掉的 `ENGINE_RUNTIME` Lazy | 删除注释块 |
| D13 | `probing/memtable/src/layout.rs` | L62–64 | 注释掉的 `FLAG_*` 常量 | 删除或实现 |
| D14 | `python/probing/repl/__init__.py` | L19 | 注释掉的 jupyter import | 删除 |

**D1–D14 粗估可删：~950–1,100 行 + ~9 个 JS 文件体积**

### 3.2 中优先级 — 合并 / 标注 / 计划移除

| ID | 路径 | 类别 | 说明 |
|----|------|------|------|
| M1 | `web/src/overhead/metrics.rs` + `web/src/pages/training.rs` + `probing/crates/skills/src/runner.rs` | 重复 | `dataframe_rows` / `df_scalar_*` / `ele_*` 三处实现 → 抽到 `web/src/utils/df.rs` 或 `probing_proto` |
| M2 | `web/src/agent/skill.rs` | 重复 | 自定义 `CatalogEntry` / `RoutingPayload` vs `probing_skills::api` |
| M3 | `web/src/agent/skill.rs` + `web/src/agent/routing.rs` | 重复 | `match_skills` / `match_intents` 与 `probing/crates/skills/src/routing.rs` 算法重复 |
| M4 | `probing/crates/skills/src/discovery.rs` ↔ `python/probing/extensions/skills.py` | 重复 | Skill root 发现双实现（Rust 还 subprocess 调 Python） |
| M5 | `probing/cli/src/cli/cluster.rs` + `cli/skill/backend.rs` + `server/src/mcp/mod.rs` | 重复 | Cluster query JSON 解析三份拷贝 |
| M6 | `python/probing/nccl/mock.py` ↔ `probing/extensions/nccl-profiler/src/tables.rs` | 漂移风险 | 列定义平行维护；mock 缺 `nccl.profiler_counters` |
| M7 | `python/probing/skills/loader.py` | API 遗留 | `root=` 参数多处 `NotImplementedError` / deprecated 文案 |
| M8 | `probing/core/src/core/federation/convert.rs` | dead_code | `node_label` 仅测试使用 → `#[cfg(test)]` |
| M9 | `web/src/api/mod.rs` | 过度 re-export | 15 个子模块 `#[allow(unused_imports)]` |
| M10 | `web/src/components/colors.rs` | dead_code | 模块级 `allow(dead_code)`；Tailwind safelist 子集 |
| M11 | `web/src/state/ui_tasks.rs` | 预留 | `UiTaskKind::Query` 标 reserved，未实现 |
| M12 | `probing/extensions/python/src/features/python_api.rs` | 不一致 | `api_eval` 直接用 `PythonRepl`；server `/ws` 已用 `ReplSession` facade |
| M13 | `examples/external_table_retirement.py` | 示例债务 | 无限循环 + 旧 API 用法；未在 `examples/README.md` 列出 |

### 3.3 低优先级 / 有意保留

| ID | 路径 | 说明 |
|----|------|------|
| L1 | `docs/src/design/architecture.md` | modularity 标 superseded；历史图，可保留 banner |
| L2 | `probing/extensions/nccl-profiler-cdylib/` + `hccl-profapi/` | 与主 crate 共享 `lib.rs`，打包名不同 — **有意** |
| L3 | `python/probing/profiling/collective/coll.py` | 标 legacy coarse；NCCL plugin 启用时默认关 — **回退路径** |
| L4 | `probing/extensions/python/src/features/spy/python_bindings/` | vendored py-spy，`allow(dead_code)` — AGENTS.md 豁免 |
| L5 | `probing/core/src/core/federation/rewrite.rs` | `rewrite_*_legacy` 为 sqlparser 失败回退 — **有意 fallback** |
| L6 | `web/src/components/flamegraph/` + `features/flamegraph.rs` | HTML 与 WASM 双渲染路径 — 若弃 CLI HTML 预览可删 ~300 行 |

### 3.4 误报澄清（看起来像死代码，实际有用途）

| 模块 | 用途 | 引用 |
|------|------|------|
| `python/probing/testing/` | `@table` / SQL 外部表回归测试工厂 | `tests/regression/tbls/test_python_tbls.py` 查 `python.probing.testing.get_*()` |
| `python/probing/timing/` | 实验性 GPU timing API | 仅 `tests/unit/probing/timing/*` 与 `tests/regression/timing/*` — **生产未用，但非死代码**；建议标 `@experimental` 或合并进 `torch_probe` |
| `python/probing/inspect/__init__.py` | 包占位 | 实际逻辑在 `inspect/trace.py` — 可合并，非删除 |
| `skills/` ↔ `bundled_skills/` | SSOT 符号链接 + wheel 复制 | 有意设计 |

---

## 4. 重复实现详表

### 4.1 Skill 生态（ROI 最高）

```
Rust SSOT (执行)          probing/crates/skills/
    ├── loader.rs         YAML 解析、模板展开
    ├── discovery.rs      skill root 发现  ←──┐ 双实现
    ├── routing.rs        match_skills       │
    └── runner.rs         SkillBackend         │
                                               │
Python (发现 + Agent)     python/probing/extensions/skills.py  ←──┘
                          python/probing/skills/loader.py (委托 _core)
Web (WASM)                web/src/agent/skill.rs (重复 routing + 类型)
                          web/src/api/skills.rs (HTTP 拉 catalog)
```

| 重复点 | 文件 A | 文件 B | 可删行数(估) | 性质 |
|--------|--------|--------|--------------|------|
| Skill root 发现 | `discovery.rs:30–197` | `extensions/skills.py:35–151` | 120–150 | **意外** — 应单侧 SSOT |
| match_skills 评分 | `routing.rs:8–80` | `web/agent/skill.rs:138–167` | 40–55 | **意外** |
| Skill JSON 重建 | `loader.rs:168–203` | `api.rs:83–246` | 60–80 | 半有意 — 可抽 builder |
| DataFrame 行解析 | `runner.rs:306+` | `overhead/metrics.rs:119+` | 30–50 | **意外** |

### 4.2 集群 / 查询路径

| 重复点 | 位置 | 可删行数(估) |
|--------|------|--------------|
| Cluster query 响应解析 | `cli/cluster.rs:39–67`, `cli/skill/backend.rs:21–41`, `server/mcp/mod.rs:263–306` | 50–70 |
| Node 列表 LLM 格式化 | `web/agent/cluster.rs:43–79`, `web/agent/skills_backend.rs:52–71` | 25 |

`parse_cluster_meta` 已在 `probing/crates/skills/src/backend.rs:30–59`，但完整 envelope 解析未共享。

### 4.3 NCCL schema 双份维护

| 表 | Rust (`tables.rs`) | Python mock (`mock.py`) |
|----|--------------------|-------------------------|
| `nccl.proxy_ops` | ✓ | ✓ 列名列表 15–34 |
| `nccl.coll_perf` | ✓ | ✓ |
| `nccl.inflight_ops` | ✓ | ✓ |
| `nccl.net_qp` | ✓ | ✓ |
| `nccl.profiler_counters` | ✓ | **缺失** |

**风险：** mock 与 prod schema 漂移；`watchdog_timeout` skill 在 macOS mock 环境可能缺表。

### 4.4 Flamegraph 双路径

| 路径 | 文件 | 行数(约) |
|------|------|----------|
| Rust HTML + JSON | `extensions/python/src/features/flamegraph.rs` | ~921 |
| Web Dioxus UI | `web/src/components/flamegraph/*` | ~1093 |

Web 已走 `/apis/torchextension/flamegraph/json`；Rust HTML 路径主要服务 CLI。若确认 CLI 不再需要 standalone HTML，可删 `render_html` 分支（~300–400 行）。

### 4.5 有意边界（不宜合并）

| 边界 | 原因 |
|------|------|
| Python `config.py` ↔ Rust `config.rs` | PyO3 FFI 薄封装 |
| `torch_probe.py` (写表) ↔ `torch.rs` (读表聚合) | 生产者 / 消费者 |
| `torchrun_cluster.rs` ↔ `torchrun_cluster.py` | Python 仅 delegate `_core` |
| `tests/fixtures/*.yaml` Rust + Python 双测 | 契约 parity 测试 |
| `STEP_MATRIX_SQL` conftest ↔ `training.rs` | 注释写明 shape 对齐 — 契约镜像 |

---

## 5. modularity.md §8 技术债对照

| 文档条目 | 状态 | 备注 |
|----------|------|------|
| Python ext → CLI (`cli_main` only) | **仍在** | Accepted；保持 import 面最小 |
| Core → NCCL/HCCL (`builtin-schema-docs`) | **仍在** | L1→L2 编译依赖；目标改为 collector hook 注册 docs |
| Server → `profiling.rs` | **已清除** ✓ | |
| Server → `PythonRepl` 内部 | **部分** | `/ws` 用 `ReplSession`；`api_eval` 仍直用 `PythonRepl` |
| Skills triple loader | **大部分完成** | Web 仍有序列化 / routing 重复 |
| Composition sprawl | **轻度** | `engine.rs` ~228 行，可接受 |
| Architecture doc superseded | **有意保留** | 加 banner 即可 |

---

## 6. 清理路线图与预估收益

### Phase 0 — 零风险 quick wins（< 1 天）

- [ ] 删 `uv.lock.2`
- [ ] 删 9 个未引用 JS bundle
- [ ] 删 Web dead 函数（D5–D10）
- [ ] 删 `discovery.py` + 更新 `extensions/README.md` 消费者表
- [ ] 修复 `bundled_web/public/index.html` 标题重复：`Probing Web InterfaceProbing Web Interface`
- [ ] 删 `engine.rs` deprecated `query()`

**收益：~200 行 Rust/TS + 1.3k 行 lock + JS 体积**

### Phase 1 — 中等工作量（2–5 天）

- [ ] 删除或 feature-gate `tracing.rs`（**最大单块**）
- [ ] 抽取共享 `DataFrame` 工具（M1）
- [ ] Web routing 委托 `probing_skills::routing`（M3）
- [ ] 统一 cluster query 响应解析（M5）
- [ ] `node_label` → `#[cfg(test)]`（M8）
- [ ] `api_eval` 统一 `ReplSession`（M12）

**收益：~400–600 行**

### Phase 2 — 架构收敛（1–2 周）

- [ ] Skill root discovery 单侧 SSOT（M4）
- [ ] NCCL mock 列从 `tables.rs` 生成或共享 manifest（M6）
- [ ] Core 去 NCCL/HCCL 默认 compile dep（modularity §8）
- [ ] 评估 Flamegraph HTML 路径是否废弃（L6）
- [ ] `python/probing/timing/` 标 experimental 或并入 torch_probe

**收益：~500–900 行 + 降低 drift**

### Phase 3 — 工具链

- [ ] 修复或文档化 `web/dist` symlink 对 `rg` 的影响（`.ignore` / CI 排除）
- [ ] `make frontend` 后自动 prune 旧 `web-dxh*.js`

---

## 7. 质量门禁建议（防复发）

1. **CI 禁止新增 `#[allow(dead_code)]`** 于 `web/src/agent/*`（NCCL non-Linux / vendored py-spy 除外）
2. **`make frontend` 后断言** bundled `index.html` 引用的 JS 与 `assets/` 文件 1:1
3. **NCCL mock schema 测试**：从 Rust `tables.rs` 导出列名清单，Python mock 自动对齐
4. **Deprecation 周期**：`loader.py` 的 `root=` 参数在下个 minor 移除，而非永久 `NotImplementedError`
5. **定期跑** `cargo udeps` / `vulture`（Python）— 当前未接入 CI

---

## 8. 附录：扫描统计

| 指标 | 数值 |
|------|------|
| Rust 源文件行数（wc 抽样） | ~173,376 |
| Python 源文件行数 | ~26,185 |
| 内置 Skills | 13 |
| 测试相关文件 | ~84 |
| PyPI 版本 | 0.2.5 (Beta) |
| `allow(dead_code)` 命中（生产路径） | ≥14 处 |
| `FIXME` / `TODO remove`（probing/python/web） | 0 |

---

## 9. 待人工确认项

以下项需运行时或产品决策，静态分析无法定论：

1. **`tracing.rs` 是否有近期接入计划？** 若无，应删而非继续占 core。
2. **CLI 是否仍需要 Rust 生成的 standalone flamegraph HTML？** 决定 L6 是否可删。
3. **`python/probing/timing/` 是否对外承诺 API？** 决定 experimental 标注 vs 删除。
4. **`examples/external_table_retirement.py`** 是否仍演示有效 discard 行为？否则移入 `scripts/` 或删除。

---

*本文件由静态审计生成；Phase 0–3 清理已于 2026-07-11 执行并通过 `make test`。*

## 10. 执行记录（2026-07-11）

| Phase | 状态 | 主要变更 | 测试结果 |
|-------|------|----------|----------|
| **0** | ✅ 完成 | 删 `uv.lock.2`、9 旧 JS bundle、`discovery.py`、Web dead 函数、deprecated `Engine::query`、修复 index.html 标题 | Rust 637 + 78 ✅；Python 403 ✅ |
| **1** | ✅ 完成 | 删 `tracing.rs`（874 行）；`DataFrame` 标量/行数方法入 `probing-proto`；`parse_cluster_query_response` 共享；`api_eval`→`ReplSession`；`node_label` 仅测试 | 同上 |
| **2** | ✅ 部分 | mock 补 `nccl.profiler_counters`；`timing` 标 experimental；Rust discovery 注释 Python SSOT；**未做** Core→NCCL 编译依赖移除（架构债，需单独 PR） | 同上 |
| **3** | ✅ 完成 | `Makefile frontend` 自动 prune 旧 bundle；新增 `.rgignore` 排除 `web/dist` | 同上 |

**未纳入本次执行（需产品决策）：** Flamegraph HTML 路径废弃、Skill root 双实现合并为单侧 SSOT、Core 去 `builtin-schema-docs` 默认依赖。

---

## 11. 后向兼容与冗余代码（Phase 4 候选，2026-07-11）

Phase 0–3 已净删 **~1,024 行**（工作区未 commit）。以下为**仍可精简**项，按风险/收益排序。

### 11.1 高优先级 — 低风险，建议下一 PR

| ID | 位置 | 约行数 | 性质 | 建议 |
|----|------|--------|------|------|
| P4-H1 | `python/probing/skills/loader.py` | ~20 | **废弃 API** | 5 个函数的 `root=` 参数仅 `NotImplementedError`；全库无调用方传 `root=` → **直接删参数与分支** |
| P4-H2 | `web/src/agent/skill.rs` | ~65 | **类型重复** | `CatalogEntry` / `IntentEntry` / `PageEntry` 与 `probing_skills::catalog::CatalogEntry` + semantic YAML 平行；可改为复用 crate 类型或 `serde_json::Value` 薄层 |
| P4-H3 | `web/src/agent/skill.rs` L138–167 + `routing.rs` L81–99 | ~80 | **逻辑重复** | `match_skills_for_query` / `match_intents` 与 `probing/crates/skills/src/routing.rs` 同算法（intent 加权 + keyword 打分）；store 已加载时应委托 `probing_skills::match_skills` 或共享 scoring 函数 |
| P4-H4 | `probing/core/.../convert.rs` `PROBE_NODE_COL` | ~10 | **列名遗留** | `_probe_node` 已不再写入 schema（单测 `index_of` 失败）；`is_federation_tag_column` 仍匹配旧名 → 移除常量与 match 分支 |
| P4-H5 | `tables.yaml` 双份拷贝 | **498** | **资源重复** | `probing/core/resources/tables.yaml` 与 `python/probing/bundled_skills/semantic/tables.yaml` **字节级相同**；`skills/` 已 symlink 到 bundled → core 侧改为 `include_bytes!` / 构建复制，只保留一份 SSOT |

**H 类合计（不含 tables.yaml 去重）：~175 行；含 tables.yaml SSOT：~673 行。**

### 11.2 中优先级 — 需产品/运维确认

| ID | 位置 | 约行数 | 性质 | 建议 |
|----|------|--------|------|------|
| P4-M1 | `probing/crates/skills/src/api.rs` `skill_from_api` + `loader.py` JSON→dataclass | ~200 | **三轨映射** | YAML 走 `loader.rs` Deserialize；HTTP API 走 `skill_from_api` 手工字段；Python 再映射一遍 → 统一为单一 `Skill` serde 结构，`skill_from_api` 改为 `serde_json::from_value` |
| P4-M2 | `probing/extensions/python/.../flamegraph.rs` `render_html` | ~300–400 / 921 | **输出格式遗留** | Web 已用 `/flamegraph/json`；CLI / pprof 仍生成 standalone HTML（见 `api-reference.md`）。若 CLI 可改为 JSON + 模板页，可大幅删 Rust HTML 生成 |
| P4-M3 | `docs/src/design/architecture.md` | ~177 | **文档 superseded** | 文首已标 Legacy；可缩为 10 行 redirect + 链到 `modularity.md` |
| P4-M4 | `examples/external_table_retirement.py` | 43 | **孤儿示例** | 无限循环 + 未列入 `examples/README.md`；移入 `scripts/` 或删除 |
| P4-M5 | `python/probing/profiling/torch_probe.py` `random:`/`ordered:` 前缀 | ~20 + 文档 | **配置兼容** | `ordered` 模式已移除，前缀仍解析为 random；成本低，可保留至下个大版本再删 |
| P4-M6 | `python/probing/profiling/collective/coll.py` | 439 | **Legacy collective** | NCCL profiler 启用时默认关闭；无 plugin 时的 coarse 回退 — **保留**，除非承诺 always-on NCCL |
| P4-M7 | `probing/server/.../cluster_fanout.rs` flat 模式 | — | **运维兼容** | `PROBING_CLUSTER_FANOUT_HIERARCHICAL=0` 扁平 fan-out；有 hierarchical 单测 — **保留** |
| P4-M8 | `probing/core/Cargo.toml` `builtin-schema-docs` 默认 feature | — | **架构债** | L1 默认编译依赖 NCCL/HCCL shim（`modularity.md` §8）— 非行数问题，需 feature 默认 off + 文档 |

### 11.3 低优先级 — 可保留

| 项 | 说明 |
|----|------|
| `python/probing/config.py` / `external_table.py` | 各 ~40 行纯转发 `_core`；公开 API 稳定面，合并收益小 |
| `python/probing/timing/` | 已标 experimental；有 4 组 unit test — 删需同步删测试 |
| `federation/rewrite.rs` legacy 字符串回退 | sqlparser 失败路径；有单测 — **有意 fallback** |
| py-spy `v3_3_7`…`v3_7_0` bindings | `pyproject.toml` 仍 `requires-python >=3.7`；若 bump 到 **3.9+** 可删数千行 vendored 绑定（**breaking**） |
| NCCL profiler non-Linux `allow(dead_code)` | 跨平台 crate 结构需要 |
| `web/src/components/colors.rs`、`ui_tasks.rs` Query 变体 | 预留 UI；可等接入或删 dead 导出 |

### 11.4 建议 Phase 4 执行顺序

1. **P4-H1** — 删 `loader.py` 的 `root=`（单文件，零行为变化）
2. **P4-H4 + P4-H5** — federation 列名 + tables.yaml SSOT（构建脚本小改）
3. **P4-H2 + P4-H3** — Web agent 与 `probing-skills` routing 合并（需 WASM 回归）
4. **P4-M1** — `skill_from_api` serde 化（Rust + Python 测试）
5. **P4-M2 / M3 / M4** — 按产品决策分批

### 11.5 预估增量收益

| 范围 | 净删行数（约） |
|------|----------------|
| Phase 4 高优先级（H1–H5） | **650–700** |
| + 中优先级 M1 + M3 + M4 | **+350–450** |
| + Flamegraph HTML（M2，若 CLI 改 JSON） | **+300–400** |
| + Python 3.9+ 去旧 spy 绑定（breaking） | **+2,000+** |

**Phase 0–3 + Phase 4-H 合计可达 ~1,700 行**（不含 breaking 的 spy 清理）。

---

## 12. Phase 4 执行记录（2026-07-11）

| 步骤 | 状态 | 变更 |
|------|------|------|
| P4-H1 | ✅ | 删 `loader.py` 全部 `root=` 参数与 `NotImplementedError` 分支 |
| P4-H4 | ✅ | 删 `PROBE_NODE_COL` / `_probe_node` 遗留常量与 match |
| P4-H5 | ✅ 已满足 | `bundled_skills/semantic/tables.yaml` 已是 symlink → `probing/core/resources/tables.yaml` |
| P4-H2+H3 | ✅ | `probing-skills/routing.rs` 导出共享 routing；Web agent 委托；删 Web `match_intents` |
| P4-M1 | ✅ | `skill_from_api` 改为 serde `SkillApiPayload` |
| P4-M3+M4 | ✅ | `architecture.md` 缩为 redirect；删 `examples/external_table_retirement.py` |

**测试：** Rust 629 unit + 78 regression ✅；Python 152 unit + 251 regression ✅（5 skipped）。

---

## 13. Phase 5 执行记录（2026-07-11）

| 步骤 | 状态 | 变更 |
|------|------|------|
| P5-1 | ✅ | `loader.py` 删 `Skill` 死字段（`requires`/`variables`/`metadata`/`triggers`） |
| P5-2 | ✅ | 抽 `_step_from_raw()`，合并 `load_skill` / `expand_skill` 重复构造 |
| P5-3 | ✅ | `discovery.rs` 仅在 Python subprocess 失败时走 `fs_skill_roots` + bundled 回退 |
| P5-4 | ✅ | 删 `skills_backend.rs` 未用 `AppError` import |
| P5-5 | ✅ | 更新 `python/probing/skills/README.md` 陈旧 shim 引用 |

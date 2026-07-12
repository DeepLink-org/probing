# Probing Web — 界面设计与代码结构

本文档描述前端界面设计原则、产品信息架构与代码组织方式，便于维护与扩展。

**技术栈**：Dioxus 0.7（WASM）、dioxus-router、Tailwind（dx 构建）、reqwest、`probing-skills`（技能执行 SSOT）、async-openai（浏览器 BYOK LLM）。

---

## 一、产品信息架构

Probing Web 是 **训练/推理现场的 live 诊断工作台**，不是 experiment tracking（MLflow/W&B）替代品。核心用户路径：

```text
现象（Dashboard / Training / Spans）
  → Investigation 上下文（step / trace / pid，URL 同步 + 页内 hint）
  → Profiling / SQL 证据
  → Investigate Agent + Skill 结构化诊断
```

### 1.1 命名消歧（侧栏 title 与文案需保持一致）

| UI 名称 | 路由 | 含义 |
|---------|------|------|
| **Spans** | `/spans`（`/traces` 遗留别名） | `python.trace_event` 层级 span，分布式追踪 |
| **Profiling · Chrome trace** 等 | `/profiling/:view` | pprof / torch 火焰图、Kineto 类 timeline（非 Spans） |
| **Python** | `/python` | 函数级 live 变量 trace（非 Spans） |
| **Investigate** | `/agent` | Skill + 可选 LLM 的诊断 Agent |

### 1.2 全局壳层（非路由）

挂载于 `App` 根节点或 `AppLayout`，任意页面可用：

| 组件 | 快捷键 / 触发 | 职责 |
|------|----------------|------|
| `AppOverlays` | 侧栏 Monitors 点击 / `file:line` | 根级 viewport overlay（任务队列、Torch overhead、源码预览） |
| `CommandBar` + `GlobalCommandPanel` | ⌘K | SQL / eval REPL |
| `AgentPanel` | ⌘J（`/agent` 全页时禁用浮层） | 右侧浮层 Agent |
| `InvestigationContextHint` | 页内（有上下文时） | 轻量提示条 + 跳转 Spans |
| `SidebarMonitors` | — | 侧栏底部紧凑摘要（Tasks + Torch overhead）；点击打开对应 overlay |
| `LlmSettingsOverlay` | Agent ⚙ | LLM API 配置（localStorage） |
| `ShortcutsHelpOverlay` | `?` | 快捷键帮助 |
| `PageContextSync` | 路由变更 | 同步 `PAGE_CONTEXT`、拉 page snapshot |
| `InvestigationUrlSync` | — | 上下文 ↔ URL query 双向同步 |
| `UiTaskRuntime` | — | 全局任务计时 tick |

**Overlay 状态机**（`state/overlays.rs`）：

```text
APP_OVERLAY: None | SourceViewer(path, line) | Monitor(Tasks | Overhead)
```

- `open_monitor_overlay` / `open_source_viewer` 统一走 `open_app_overlay`，并 `lock_body_scroll`。
- 渲染入口：`components/app_overlays.rs`（`App` 根挂载，与 `Router` 并列）。
- 视觉壳：`components/overlay_shell.rs`（居中 modal；局部 Esc 关闭）。

---

## 二、界面设计原则

### 2.1 布局

- **整体**：左侧固定侧栏 + 右侧主列（CommandBar → 主内容）；侧栏可收起（localStorage）、可拖拽宽度（200–600px）。
- **标准主内容**：`max-w-7xl mx-auto`，`p-4 sm:p-6`，背景 `bg-gray-50`（与根容器一致）。
- **Fullscreen 主内容**：`AppLayout { fullscreen: true }` 时去掉 `max-w-7xl`，内层 `w-full h-full min-h-0`，用于 Profiling、Spans 等可视化页。padding 仍保留（后续可改为真正 edge-to-edge）。
- **响应式**：主列随侧栏显隐自适应；表格/火焰图/时间线支持横向滚动；窄屏侧栏为 fixed 宽度（暂无 drawer 模式）。

### 2.2 色彩（`components/colors.rs`）

- **侧栏**：深色 slate 渐变 + 蓝色强调（`SIDEBAR_*` 常量）。
- **主内容**：浅灰底、白/灰卡片、gray 文字层级；调查上下文用 blue-50 条。
- **强调**：蓝色主操作；成功/错误/警告用 green/red/amber 常量。
- **约定**：新 UI 优先从 `colors.rs` 取 Tailwind 类名字面量；Agent 新面板可用 `workspace/surface.rs` 的 `SurfaceCard`。

### 2.3 页面与状态组件

**经典页面模式**（Dashboard、Cluster、Analytics 等）：

- `PageContainer` + `PageTitle` + 若干 `Card` / `StatCard`。
- 异步数据：`AsyncBoundary` + `use_app_resource`；poll 页配合 `use_poll_tick_gated` + `PollStatusBar`。

**Workspace 模式**（Agent、部分新面板）：

- `workspace/panel_shell.rs`、`surface.rs`、`split.rs` — 统一 Agent 与浮层视觉。

**反馈状态**（`components/common.rs`）：

- `LoadingState` / `SuspenseBoundary` / `ErrorState` / `EmptyState` / `AppErrorDisplay`。

### 2.4 侧栏结构

```text
Logo
├── Overview: Dashboard, Investigate, Stacks▾
├── Analysis: Profiling▾, Analytics, Spans, Training, Pulsing
├── System: Cluster, Python
nav（flex-1 滚动）
Monitors: Background tasks · Torch overhead（摘要行，点击打开 overlay）
GitHub footer
```

### 2.5 键盘快捷键（`keyboard_shortcuts.rs`）

| 键 | 动作 |
|----|------|
| ⌘K / Ctrl+K | 打开 Command Panel |
| ⌘J / Ctrl+J | 切换 Agent 浮层（非 input focus） |
| `?` | 快捷键帮助 |
| Esc | 关闭最顶层 overlay：Shortcuts → Command → Agent → SourceViewer |

**注意**：Tasks / Overhead **monitor overlay** 由 `OverlayShell` 内 Esc 关闭；全局 Esc 链目前**不包含** `APP_OVERLAY::Monitor`（见 §九）。

---

## 三、Investigation 上下文

跨页共享的调查上下文，驱动 Agent LLM grounding、URL 深链、Spans 过滤同步。

| 模块 | 路径 | 职责 |
|------|------|------|
| 状态 | `state/investigation.rs` | `INVESTIGATION_CONTEXT`（step、trace_id、span、pid 等） |
| URL | `state/investigation_url.rs` | query 参数读写、与 localStorage 同步 |
| 提示 | `components/investigation_context_hint.rs` | 页内空状态 / 上下文引导 |

**写入入口示例**：Training 热力格、Spans 过滤、Dashboard 线程行、Agent step 导航。

**Agent 页面上下文**（与 investigation 独立）：`state/page_context.rs` + `PageContextSync` + `agent/page_tools.rs`（route snapshot 供 LLM）。

---

## 四、全局任务队列（`state/ui_tasks.rs`）

浏览器内可取消的异步任务 registry。

| 概念 | 说明 |
|------|------|
| `UiTaskHandle` | 单任务；`is_cancelled()` / `finish()` / `fail()` / `cancel()` |
| `UiTaskSession` | Agent / skill 会话组；取消任一项可取消整组 |
| `open_ui_task` | 独立任务（如 snapshot） |
| `begin_snapshot_task` | 路由切换 supersede 上一个 snapshot |
| `ui_agent_busy()` | Agent 输入禁用、chip disabled |
| `UI_TASK_TICK` | 500ms tick，驱动侧栏 Monitors elapsed 显示 |

**任务种类**（`UiTaskKind`）：`Agent` · `Snapshot` · `Skill` · `Query`（Query 预留，Command Panel 待接入）。

**UI 入口**：侧栏 `SidebarMonitors` 摘要 → `AppOverlays::TasksMonitorOverlay` 全屏列表（可 Cancel all / Clear finished）。

---

## 五、Torch Overhead 监控

侧栏与 overlay 共用的 TorchProbe 开销视图（与 `docs/src/design/overhead-invariants.*` 对齐）。

| 层 | 路径 | 职责 |
|----|------|------|
| 领域 | `overhead/metrics.rs` | `OverheadSnapshot`、等级判定、侧栏文案 |
| SQL | `overhead/sql.rs` | 固定窗口 SQL（`WINDOW_STEPS=80` 等常量） |
| API | `api/overhead.rs` | `fetch_overhead_summary`、NCCL counters 可选 |
| UI | `components/overhead/panel.rs` | `TorchOverheadPanel` 表格与脚注 |
| 侧栏 | `components/sidebar/monitors.rs` | 轮询摘要 + 打开 `OverheadMonitorOverlay` |

轮询间隔：`OVERHEAD_POLL_MS`（2000ms），页面不可见时 `use_poll_tick_gated` 暂停。

---

## 六、Agent 与 Skills

Web **不嵌入** skill YAML；启动时从 probing server 拉取，执行走共享 Rust crate `probing-skills`。

| 模块 | 路径 | 职责 |
|------|------|------|
| UI | `components/agent/` | `chat.rs`（全页 + 浮层）、`step_card.rs`、`panel.rs`、`settings.rs` |
| 加载 | `agent/skill.rs` | 内存 store；`populate_skill_store` |
| API | `api/skills.rs` | `GET /apis/pythonext/skills/routing` + per-id `load` |
| 路由 | `agent/routing.rs` | `Route` → page_id；catalog / intents / pages 来自 server |
| 执行 | `agent/runner.rs` | `run_skill` → `probing_skills::run_step` |
| 后端 | `agent/skills_backend.rs` | `WebBackend`：`POST /query`、`/apis/cluster/query`、GET API |
| 解释 | `agent/interpret.rs` | 桥接 `probing-skills` interpret 类型 |
| LLM | `agent/llm.rs` | `select_skill`、`summarize_run`（async-openai） |
| 状态 | `state/agent.rs` | 消息、输入、浮层宽度 |

**数据流**：

```text
AppLayout mount → ApiClient::load_skill_store()
  → /apis/pythonext/skills/routing + /load?id=…
  → agent/skill.rs STORE
Agent chip / LLM → resolve_skill_id → run_skill(WebBackend)
  → probing-skills runner (与 CLI / MCP 同语义)
```

Skill 内容 SSOT：仓库根 `skills/`（wheel：`python/probing/bundled_skills/`）。详见 `skills/README.md`、`AGENTS.md`。

---

## 七、代码结构

```
web/src/
├── main.rs                 # WASM 入口；支持 base_path 子路径部署
├── app.rs                  # Route 枚举 + 页面包 AppLayout + App 根 AppOverlays
├── api/                    # ApiClient 与分域 endpoint（含 skills、overhead）
├── agent/                  # Skill 加载、LLM、runner、WebBackend、routing
├── overhead/               # Torch overhead 领域逻辑（metrics + SQL，无 UI）
├── hooks/mod.rs            # use_app_resource（首选）、use_api（遗留）、poll 辅助
├── pages/                  # 业务页
│   ├── dashboard.rs
│   ├── agent.rs
│   ├── profiling.rs
│   ├── traces.rs           # Spans（/spans、/traces redirect）
│   ├── training.rs
│   ├── analytics.rs
│   ├── stack.rs
│   ├── python/
│   ├── pulsing.rs
│   └── cluster.rs
├── state/
│   ├── investigation.rs
│   ├── investigation_url.rs
│   ├── page_context.rs
│   ├── ui_tasks.rs
│   ├── overlays.rs         # APP_OVERLAY 状态机
│   ├── scroll_lock.rs      # overlay 打开时锁定 body 滚动
│   ├── profiling.rs
│   ├── stack.rs
│   ├── agent.rs
│   ├── profile_snapshots.rs
│   ├── sidebar.rs
│   ├── commands.rs
│   ├── llm_config.rs
│   └── source_viewer.rs    # 薄封装，转发 overlays API
├── components/
│   ├── layout.rs           # AppLayout 壳
│   ├── app_overlays.rs     # Tasks / Overhead / SourceViewer 根渲染
│   ├── overlay_shell.rs    # 居中 modal 壳
│   ├── sidebar/            # 导航、Monitors、Profiling/Stack 子菜单、resize
│   ├── overhead/           # TorchOverheadPanel（UI）
│   ├── workspace/
│   ├── agent/
│   ├── flamegraph/
│   ├── timeline_viewer/
│   ├── source_viewer.rs
│   ├── global_command_panel.rs
│   ├── investigation_context_hint.rs
│   ├── profile_snapshot_bar.rs
│   ├── page_context_sync.rs
│   ├── ui_task_runtime.rs
│   ├── keyboard_shortcuts.rs
│   └── ...
└── utils/
```

### 7.1 约定

| 主题 | 约定 |
|------|------|
| **新页面** | 放 `pages/`，在 `app.rs` 注册 `Route` + 路由组件 |
| **跨页状态** | `state/` GlobalSignal；避免在 render 分支内 `write()`（用 `use_effect`） |
| **拉数** | 新代码用 `use_app_resource` + `AsyncBoundary`；`use_api` 仅遗留页（如 Pulsing） |
| **样式** | `colors.rs` 常量 > 硬编码 Tailwind |
| **错误** | `utils/error.rs` 的 `AppError` + `display_message()` |
| **Skills** | 改 `skills/<id>/` + `python -m probing.skills validate`；Web 运行时从 server 加载 |
| **新 overlay** | 扩展 `AppOverlay` 枚举 + `app_overlays.rs` 分支；优先复用 `OverlayShell` |
| **WASM 限制** | `dioxus-code` 仅 native；Source viewer 为 plain text + 行号 gutter |

### 7.2 组件职责（扩展）

| 模块 | 职责 |
|------|------|
| `layout` | 侧栏、CommandBar、主内容区、Agent 浮层容器；启动 `load_skill_store` |
| `sidebar` | 导航、Monitors 摘要、Profiling/Stack 控件、resize |
| `app_overlays` | 根级 Tasks / Overhead / SourceViewer |
| `flamegraph` | pprof / torch 火焰图、diff |
| `timeline_viewer` | trace / pytorch / ray timeline |
| `profile_snapshot_bar` | 火焰图 capture + baseline diff（会话内内存） |
| `callstack_view` | 混合栈 + SourceLocationLink |
| `source_viewer` | 源码 modal（经 `APP_OVERLAY`） |
| `global_command_panel` | ⌘K REPL |
| `dataframe_view` / `table_view` | 表格展示 |
| `poll_status` | 轮询状态条 |

---

## 八、路由一览

| 路由 | 页面 | AppLayout |
|------|------|-----------|
| `/` | Dashboard | 标准 |
| `/agent` | Investigate（全页 Agent） | 标准 |
| `/cluster` | Cluster | 标准 |
| `/stacks`, `/stacks/:tid` | Stacks | 标准 |
| `/profiling`, `/profiling/:view` | Profiling | **fullscreen** |
| `/analytics` | Analytics SQL | 标准 |
| `/python` | Python variable trace | 标准 |
| `/spans`, `/traces` | Spans（`/traces` → redirect） | **fullscreen** |
| `/training` | Training 热力图 / collective | 标准 |
| `/pulsing` | Pulsing actors | 标准 |
| `/chrome-tracing` | → redirect `/profiling/trace` | — |

---

## 九、构建与部署

- 开发 / 构建：`dx serve` / `dx build --release`；仓库根 `make frontend` 复制产物到 `web/dist/`。
- UI 静态资源由 Python 包提供（wheel：`python/probing/_web/`；editable：`web/dist/`），经 `probing.web_assets` 设置 `PROBING_ASSETS_ROOT`，`probing-server` 只读该目录；未配置时返回占位页。

---

## 十、已知差距与后续方向

以下为当前实现与理想状态之间的差距，供迭代参考（非阻塞发布）：

**已修复（历史 P0）**：`pages/stack.rs` 侧栏帧计数改为 `use_effect`；删除 `chrome_tracing_iframe`；Playbook 体系迁移为 Skills + `probing-skills`。

**重构后已落地**：`SidebarMonitors` + `AppOverlays`；`overhead/` 领域模块；runtime skill 加载；`OverlayShell` 统一 modal。

1. **全局 Esc 与 monitor overlay** — `keyboard_shortcuts.rs` 未调用 `close_app_overlay()`；Tasks/Overhead 仅依赖 `OverlayShell` 局部 Esc。
2. **`/traces` 与 `/spans` 重复** — 侧栏仅推广 `/spans`；`/traces` 保留兼容 redirect。
3. **Fullscreen padding** — Profiling/Spans 仍带 `p-4 sm:p-6`，未完全 edge-to-edge。
4. **Profile snapshots** — 仅内存，刷新丢失；可接 sessionStorage。
5. **Pulsing** — 仍用 legacy `use_api`，未接 investigation / poll。
6. **`ui_tasks::Query`** — Command Panel eval 等待接入任务队列。
7. **移动端** — 无侧栏 drawer；Agent 浮层窄屏体验待优化。
8. **W&B / MLflow** — 互补集成（run 关联、诊断写回），非 UI 内建 experiment tracking。

---

## 十一、相关文档

| 文档 | 位置 |
|------|------|
| Skills 格式与 catalog | `skills/README.md`、`skills/catalog.yaml` |
| Agent / MCP 集成 | 仓库根 `AGENTS.md` |
| TorchProbe / overhead 不变量 | `docs/src/design/overhead-invariants.zh.md` |
| Profiling / TorchProbe | `docs/src/design/profiling.zh.md` |
| 扩展与自定义表 | `docs/src/design/extensibility.zh.md` |
| 训练调试示例 | `docs/src/examples/training-debugging.zh.md` |
| HTTP / skills API | `probing/server/API.md` |

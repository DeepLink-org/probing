# Probing Web — 界面设计与代码结构

本文档描述前端界面设计原则、产品信息架构与代码组织方式，便于维护与扩展。

**技术栈**：Dioxus 0.7（WASM）、dioxus-router、Tailwind（dx 构建）、reqwest、`probing-skills`（技能执行 SSOT）、async-openai（浏览器 BYOK LLM）。

## 双 UI 渐进迁移

`main.rs` 通过 `ui_version.rs::RootApp` 只挂载一个应用根：

- `next`（默认）：`next::NextApp`，独立 Router、Shell、信息架构和诊断首页。
- `classic`（冻结回退）：原有 `app::App`，路由和组件保持不变。

未保存过界面偏好的用户直接进入 Next。已有偏好继续生效；Classic 用户可通过右下角入口进入 Next，
Next 用户通过侧栏底部切回 Classic，也可使用 `?ui=classic|next` 手动切换。选择保存在
`localStorage["probing.ui.version"]`。切换时整页重载，避免两个 Router、hook
和全局监听器同时存在。

Next UI 代码边界：

```text
web/src/next/
├── routes.rs       # 独立 NextRoute
├── page_registry.rs # 页面身份、布局、侧栏分组、证据与调查上下文合同
├── shell.rs        # 诊断优先的导航与任务上下文
├── components.rs   # Next 专用页面原语
├── model.rs        # 首页/分布式健康派生模型
└── pages/          # Dashboard、Investigate、Training、Distributed、Profiles、Explore
```

Next Router 保持 Classic 产品 URL 的兼容性，并在新壳层中直接承载成熟能力：

| 工作区 | 路由 |
|--------|------|
| 诊断 | `/`、`/agent`、`/training`、`/distributed` |
| RL / 推理 | `/rl`、`/rl/train`、`/rl/spans`、`/rl/process-timeline`、`/rl/perfetto`、`/rl/inference` |
| 证据 | `/spans`、`/stacks/*`、`/profiles`、`/profiling/:view` |
| 工具 | `/analytics`、`/python`、`/pulsing`、`/cluster`、`/system` |

Next Shell 同时挂载通过 `⌘K` 唤起的 Command Panel、全局快捷键、Investigation
URL 同步、页面发布的 evidence snapshot、后台任务与 Torch overhead monitor，以及可浮动的
Investigate 面板。主内容区不设置固定顶栏，运行上下文和操作由当前页面按需承载。
低频命令输入不常驻占用页面纵向空间。
Classic 继续作为独立应用保留；已知产品路由不再依赖 Classic fallback。

### 迁移边界与顺序

Next 页面采用平行迁移，不在 Classic 页面中增加 Next 分支：

- `src/pages/` 是 Classic 冻结基线；迁移期间只接受 Classic 自身的阻断性修复。
- `src/next/pages/` 拥有 Next 页面、状态组合和交互。
- `src/api/`、协议 DTO 与纯派生模型可以共享；已完成迁移的 Next 页面不得再挂载 Classic 页面组件。
- 所有已知产品路由已解除 Classic 页面挂载；`ClassicFallback` 只承接未知或历史 URL，并引导用户回到 Next 能力目录。

迁移台账：

| 顺序 | 工作区 | 当前状态 |
|------|--------|----------|
| 1 | Dashboard | Next 原生 |
| 2 | Cluster Overview / Nodes / Distributed Status | Next 原生 |
| 3 | Training / Inference / RL | Next 原生 |
| 4 | Profiling / Stacks / Tracing | 已在 Next 原生实现 |
| 5 | Analytics / Python Trace / Pulsing / System | 已在 Next 原生实现 |
| 6 | Investigate | Next 原生；保留与 Skill/Agent 合同的持续能力对照 |
| 7 | Shell / Classic retirement | Next 已成为默认入口；Classic 继续冻结并保留显式回退，待独立退休阶段删除 |

---

## 一、产品信息架构

Probing Web 是 **训练/推理现场的 live 诊断工作台**，不是 experiment tracking（MLflow/W&B）替代品。核心用户路径：

```text
现象（Dashboard / Training / Spans）
  → Investigation 上下文（step / rank / host / trace / pid / tid，URL 同步 + 固定调查条）
  → Profiling / SQL 证据
  → Investigate Agent + Skill 结构化诊断
```

### 1.1 命名消歧（侧栏 title 与文案需保持一致）

| UI 名称 | 路由 | 含义 |
|---------|------|------|
| **Tracing** | `/spans`（`/traces` 遗留别名） | `python.trace_event` 层级 span，分布式追踪 |
| **Profiling · Chrome trace** 等 | `/profiles`、`/profiling/:view` | pprof / torch 火焰图、Kineto 类 timeline（非 Tracing） |
| **Python** | `/python` | 函数级 live 变量 trace（非 Spans） |
| **Investigate** | `/agent` | Skill + 可选 LLM 的诊断 Agent |

### 1.2 全局壳层（非路由）

挂载于 `App` 根节点或 `AppLayout`，任意页面可用：

| 组件 | 快捷键 / 触发 | 职责 |
|------|----------------|------|
| `AppOverlays` | 侧栏 Monitors 点击 / `file:line` | 根级 viewport overlay（任务队列、Torch overhead、源码预览） |
| `GlobalCommandPanel` | 侧栏搜索 / ⌘K | SQL / eval REPL；不常驻输入条 |
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

- **整体**：左侧固定功能工作区 + 右侧主内容；不设置全局固定顶栏。
- **侧栏功能聚焦**：Next 侧栏由 56px 固定功能轨道和当前功能详情区组成，但整体仍是一个侧栏。Dashboard、Investigate、Cluster、Training、Inference、RL、Profiling、Stacks、Tracing、Deep tools 图标始终保持固定顺序和坐标；切换页面只替换详情区，不移动轨道。详情区承载当前功能名称、子视图和控制项，不再把控制面板插入导航树，也不依赖 hover 承载可编辑参数。
- **Dashboard 证据平面**：Dashboard 不生成“健康”“异常”或“下一步建议”等自动结论，只在一个主证据平面内提供三组可直接比较的事实：Step time 展示 latest step、median、P95、maximum 与近期 median/P95 曲线；GPU load 展示设备平均值和逐设备 utilization/memory；Latest rank step time 展示 rank 覆盖、失败节点、median 参考线及最慢 rank 的原始时长。区域之间使用分隔线而不是等权卡片。点击 rank 行会固定 rank/step 调查上下文。Step time 与 rank comparison 来自同一次 cluster fan-out，GPU load 明确保持 process-local；两种 Scope 不混合成同一个统计量。缺失采集数据只陈述缺失范围，不从缺失推断工作负载状态。
- **Training 证据平面**：页面不使用流程提问或结论式引导；Step time、Placement、Module Hotspots、Collective Communications 是同一主证据平面中的四个分区。Step time 只保留 Latest / Average / P95 / Maximum 与趋势折线。Placement 只显示 heartbeat 实际上报的 host/rank，且仅保留微型 Overview：一格代表一张 GPU，单机按 local rank 组成 `8×1` 纵列，桌面端每行最多 8 台主机，超过后换行。悬停只预览该 rank 的 TP / DP / PP 通信组；点击固定 rank 后，在同一证据区联动显示节点 endpoint、上报状态、heartbeat 新鲜度、该 rank 最新 step 相对当前可比 rank 中位数的差值、设备与 PyTorch allocator 两层显存证据，以及 TP / DP / PP 三个通信组的成员与性能。设备层通过 `gpu.utilization` 显示当前 used、最近 5 分钟采样峰值、capacity、headroom 和样本数；allocator 层严格区分最新 `post *` hook 的 `allocated`、自 allocator reset 以来的 `max_allocated` 和当前 `cached/reserved`，并给出 peak−current、reserved−allocated 与 allocated/reserved。两层口径不互相替代，也不把 allocator 占用误写成整卡显存利用率。通信组性能只接受 `participate_ranks` 与当前组成员完全一致的样本，并明确标注 Torch API wall time、采样 rank 覆盖和数据范围；缺少样本时保持 unknown，不从 topology 推测性能。Module、Collective、Placement 的通信与显存子证据均保留 fan-out receipt；任何失败 peer 或 partial response 都必须在对应分区可见，不能在传给可视化组件时解包丢失。固定的 rank、host 和当前 step 可通过全局调查条继续进入 Tracing、Stacks、Profiling 或 Investigate。仅在 `_role` 含 `dp/pp/tp/cp/ep` 坐标时展示并行坐标，不从 world size 猜测并行策略。当 heartbeat 未上报或 registry 请求失败时，主内容不渲染空 Placement 分区；左侧 Training 控制区显示缺失原因和重试语义，数据恢复后提示自动消失。
- **Stacks 证据平面**：Local stacks 明确标注一次 on-demand capture 的 thread 和 root→current 顺序；Captured evidence 与 Call hierarchy 使用同一主证据平面和内部区隔。Call path 支持按函数/源码搜索，语言过滤保留在侧栏，帧默认紧凑并优先展开 current frame。Distributed stacks 在同一平面中展示覆盖摘要与 flamegraph；部分 fan-out 结果继续渲染，并明确缺失 peer，不把部分结果伪装成完整集群结论。
- **Spans / Timeline 树状交互**：Spans 与 Chrome trace 默认都使用稳定的聚合时间树。Tracing 先按 `trace_id` 分组；Trace 折叠行保留 roots、spans、threads、events、active、root occupancy union、窗口跨度与 self time 汇总，再按真实父子关系逐级展开 Span。Span 行固定为 `Structure / Position & occupancy / Total / Self / Cover`；折叠时保留 nested/event 数、thread、total、self、窗口覆盖率，并在父 span 的真实时间条上叠加直接子节点 occupancy，用户无需展开也能看到子树结构。展开仅分解真实父子关系和 events；高频 attributes 不自动占行，只在点击明确的 `meta` 控件后展示。Chrome trace 使用 `track → 同名同类事件组 → 单次 slice → children`，同样保留 occurrence 数、nested 数和时间 summary。`Expand all / Collapse all` 同步控制所有层级。Chrome Timeline 保留原横向轨道、缩放与 Perfetto 导出作为 `Timeline` 模式。各模式使用同一份采集数据且不生成自动瓶颈结论。
- **标准主内容**：Next Shell 统一使用 `max-w-[1600px] mx-auto` 与 `p-4 lg:p-5`，背景 `bg-gray-50`。
- **Full-height 主内容**：Profiling、Chrome trace、Perfetto 由 Shell 统一提供 `h-full min-h-0` 工作区；页面不得自行计算 `100vh` 或重复套卡片边框。
- **桌面布局**：展开时侧栏总宽 288px（56px 轨道 + 232px 详情），收起时只保留 56px 轨道；状态写入 `localStorage["probing_next_sidebar_compact"]`。本阶段不设计移动端 drawer。

### 2.1.1 Next 页面语言

所有 Next 产品页使用同一组页面原语，避免页面迁移后重新长出不同的视觉和交互方言：

- `WorkspacePage` 统一页面标题、范围说明、直接操作和 16px 内容节奏；页面不得自行复制 header。
- `EvidenceSurface` 表达页面的主证据平面；相关证据优先放在同一平面内，避免每个区块都成为等权卡片。
- `EvidenceSection` 使用标题、说明和细分隔线组织主证据平面。`SectionCard` 只保留给边界独立、可单独加载或需要独立滚动的证据范围。
- `EvidenceMetric` 只用于卡片内部的紧凑事实摘要，统一 label、数值、detail 和 tabular number 样式；跨页面不再定义私有 metric 组件。
- `FilterInput` 只过滤当前已加载证据；路由选择和采集配置继续归侧栏，避免页面工具条同时承担导航与配置。
- `ActionButton` 只用于执行、展开、导出等页面内动作，并统一 primary / neutral / danger 权重；路由跳转继续使用 `Link`。
- 请求失败、成功空结果和部分结果必须分别展示。`InlineNotice` 只能陈述覆盖率、缺失范围或请求状态，不使用“健康”“异常”“建议下一步”等替用户作结论的措辞。

信息层级固定为：页面范围 → 固定调查上下文 → 主证据平面 → 分区 summary → 图表/树/表格原始证据。没有信息增量的介绍条、流程提问、重复 overview 和纯装饰卡片不进入主内容区。

### 2.2 色彩（`components/colors.rs`）

- **侧栏**：深色 slate 渐变 + 蓝色强调（`SIDEBAR_*` 常量）。
- **主内容**：浅灰底、白/灰卡片、gray 文字层级；调查上下文用 blue-50 条。
- **强调**：蓝色主操作；成功/错误/警告用 green/red/amber 常量。
- **约定**：新 UI 优先从 `colors.rs` 取 Tailwind 类名字面量；Agent 新面板可用 `workspace/surface.rs` 的 `SurfaceCard`。

### 2.2.1 可访问性基线

- Next 主信息、表格、标签和控制项不低于 `text-xs`（12px）；只有图表刻度等辅助图形文字可以使用 11px，不使用 7–10px 承载诊断信息。
- 白色或 gray-50 主内容上的辅助文字至少使用 gray-500；深色侧栏的辅助文字至少使用 slate-400。
- 颜色不能成为唯一编码：状态需要文字，Placement 通信组同时使用颜色、虚线和 `T / D / P / ●` 字符，选中项显示 `Selected` 或 `Pinned`。
- Hover 只用于预览。相同信息必须可通过键盘 Focus 获得，通过 Click 固定，并在可见详情区或调查上下文条中保持。
- 路由、按钮、树节点和可展开帧必须有可见的 `focus-visible` 状态；当前导航使用 `aria-current`，Toggle 使用 checked/pressed 语义。
- 图表和时间条提供文本摘要或 `aria-label`；纯装饰色块对屏幕阅读器隐藏。动画遵守 `prefers-reduced-motion`。
- Shell 提供 Skip Link；主内容可直接获得焦点，不要求键盘用户遍历整个侧栏。

### 2.3 页面与状态组件

**经典页面模式**（Dashboard、Cluster、Analytics 等）：

- `PageContainer` + `PageTitle` + 若干 `Card` / `StatCard`。
- 异步数据：`AsyncBoundary` + `use_app_resource`；poll 页配合 `use_poll_tick_gated` + `PollStatusBar`。

**Workspace 模式**（Agent、部分新面板）：

- `workspace/panel_shell.rs`、`surface.rs`、`split.rs` — 统一 Agent 与浮层视觉。

**反馈状态**（`components/common.rs`）：

- `LoadingState` / `SuspenseBoundary` / `ErrorState` / `EmptyState` / `AppErrorDisplay`。

### 2.4 侧栏结构

Next 侧栏以用户角色和分析深度组织，固定轨道负责选择功能，详情区负责当前功能内部导航与参数：

```text
56px 固定轨道                    当前功能详情（展开时 232px）
Logo                            分类 + 当前功能标题 + 收起按钮
Search（⌘K）                   ├── 子视图（若存在）
Dashboard                       ├── 当前页面控制项
Investigate                     └── 可滚动的功能上下文
Cluster                          ├── Overview
                                 ├── Nodes
                                 └── Distributed Status
── Workloads: Training / Inference / RL
── Advanced: Profiling / Stacks / Tracing
── Deep tools
Tasks / Overhead / Classic      紧凑状态行
```

活动路径同时承载当前页面的控制项，例如刷新、数据范围、cluster fan-out、
采样频率和 profiler 启停；右侧页面只保留当前判断、关键指标、证据和直接下一步。
功能切换不会改变轨道中其他图标的位置。子视图导航与参数控制只出现在详情区，
因此不存在“点击标题究竟是导航还是展开”的混合语义，也不使用 `More` hover
面板。桌面侧栏支持 288px 控制模式和 56px 图标模式。

Classic 侧栏保持原结构：

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
| 状态 | `state/investigation.rs` | `INVESTIGATION_CONTEXT`（step、rank、host、trace_id、span、pid/tid） |
| URL | `state/investigation_url.rs` | query 参数读写、与 localStorage 同步 |
| 提示 | `components/investigation_context_hint.rs` | 页内空状态 / 上下文引导 |

固定上下文以紧凑的 blue-50 调查条显示，字段写入 URL 和 localStorage；调查条、侧栏轨道和侧栏子视图都使用包含完整坐标的 durable link，普通跳转、复制链接和新标签页不得丢失上下文。Hover 只做预览，Click/键盘选择才固定上下文。

调查条只表示用户固定的坐标以及页面是否具备使用该坐标的证据，不代表每个面板都已经成功筛选。面板必须根据实际查询结果区分 `matched`、`out of scope`、`no matching sample` 和 `unsupported`；找不到 Rank/Host/GPU 时禁止静默回退到其他设备。集群覆盖率同时区分 heartbeat 注册 Rank、返回 step 样本的 Rank 和失败 peer endpoint，不能把 endpoint 数量当成 world size。

Cluster Nodes 在 registry 表格上提供 Rank、Host、GPU、endpoint、role 的本地筛选，并可只显示固定进程；Profiler 控制项必须同时陈述 Scope、设置生效时机和 disabled/enabled 状态，不能只暴露一个缺少动作语义的数值控件。

**写入入口示例**：Dashboard 慢 rank 行、Training placement GPU、Tracing trace/span 行、Dashboard 线程行、Agent/SQL 结果行。

**Agent 页面上下文**（与 investigation 独立）：Next 页面以 `EvidenceRequest → EvidencePayload<T> → EvidenceBundle` 传递 scope、采样时间、行数、fan-out peer 与 partial 状态。Dashboard、Training、Memory 将当前 UI 实际渲染的 payload 发布到 `PAGE_CONTEXT`，Agent 直接消费同一份 bundle；面板关闭时不发起 snapshot 请求。证据身份使用完整的 pid/tid/rank/host/GPU/step/trace/span coordinate key，不使用可能重复的友好 label；LLM grounding 同时保留友好摘要和完整坐标。尚未接入页面发布的路由只在 Agent 打开或用户显式刷新时执行 route snapshot fallback。迟到的旧 route、旧调查坐标和旧请求不会覆盖当前页面证据。

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
| `layout` | 侧栏、运行上下文、主内容区、Agent 浮层容器；启动 `load_skill_store` |
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
| `/distributed` | Cluster Overview | 标准 |
| `/cluster` | Cluster Nodes | 标准 |
| `/cluster/status` | Distributed Status（Wait Counters / Rendezvous） | 标准 |
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

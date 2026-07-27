# Probing 项目设计与实现评审报告

## 一、项目概览

**Probing** 是一个面向分布式 AI 训练的 Agent-Native 性能诊断系统。它通过 Rust 引擎提供 SQL 查询能力，通过 Python 插件实现训练侧数据采集，通过联邦查询实现跨 rank 诊断，并通过 Skills 系统将诊断知识固化为可执行的自动化流程。

| 维度 | 数据 |
|------|------|
| Rust 代码（不含 web） | ~86,290 行 |
| Python SDK | ~15,188 行 |
| Web UI (Dioxus WASM) | ~21,479 行 |
| 测试代码 | ~7,694 行 |
| Skills 定义 (YAML+MD) | ~3,555 行 |
| Workspace crate 数 | 15 |
| 诊断 Skill 数 | 13 |
| 当前版本 | 0.2.5 (Beta) |

---

## 二、架构设计评价

### 2.1 四层分层架构

项目定义了清晰的四层架构模型：

```
L4 Experience  ─  skills/ · web/ · Python hooks (UX, 诊断剧本)
L3 Control     ─  probing/server · probing/cli (HTTP, MCP, inject, fan-out)
L2 Collectors  ─  probing/extensions/* · python/probing/profiling (指标采集)
L1 Platform    ─  probing/core · memtable · proto (SQL引擎, 联邦, 存储格式)
```

**亮点：**
- 依赖方向严格向下流动，proto 不依赖任何上层，core 不依赖 collectors
- 组合根（composition root）只存在于 `probing/server/src/engine.rs`，不在 L2 层散布组装逻辑
- `modularity.md` 文档以表格形式追踪已知技术债务，标注"Done"/"Accepted"状态
- 6 个公共接口契约（`ProbeDataSource`、`ProbeExtension`、`@table`、`skills/*/steps.yaml`、proto DTOs、Federation tags）定义了模块间的交互边界

**不足：**
- `probing-python` 扩展以 53K 行占据全部 extensions 的 84%，承载了 spy/crash/pprof/torch/tracing/repl/flamegraph 等多个独立功能域，内部高内聚但 crate 级别缺乏拆分
- modularity.md 中将此标记为 "Accepted"（maturin 单 wheel 约束），属于架构决策而非疏忽

### 2.2 核心引擎设计

**DataFusion 作为 SQL 引擎**是最重要的架构决策。不重新发明 SQL parser/optimizer/executor，而是嵌入 Apache DataFusion，所有自定义表只需实现 `TableProvider` trait。这使用户可以使用标准 SQL（JOIN、窗口函数、CTE、子查询）查询运行时数据。

`EngineBuilder` 采用 Builder 模式链式构建引擎，`ENGINE: Lazy<RwLock<Engine>>` 作为进程级单例。两个核心 trait 分离关注点：
- `ProbeDataSource`：注册 SQL 表（数据面）
- `ProbeExtension`：提供配置 + HTTP API（控制面）

### 2.3 联邦查询系统

联邦模块（~3,066 行）是项目最具技术深度的部分，采用三路径路由策略：

| 路径 | 适用场景 | 机制 |
|------|---------|------|
| Path A: AggregatePushdown | 单表 `global.*` + merge-safe 聚合 | 将 SUM/COUNT/MIN/MAX 下推到各 rank，合并结果 |
| Path B: FederatedScan | 单表 `global.*` 非 pushdown | Lazy 分区扫描，流式拉取 |
| Path C: Broadcast | JOIN/CTE/UNION/子查询 | 全量广播到每个 rank，本地执行后合并 |

路由基于 `sqlparser-rs` AST 级别分析，而非字符串匹配。查询护栏强制 broadcast 路径需要 LIMIT，自动为无 LIMIT 的 federated scan 注入 `LIMIT 10000`。每行联邦数据自动附加 6 个标签列（`_host`, `_addr`, `_rank`, `_node_rank`, `_local_rank`, `_role`）。

### 2.4 Memtable 存储格式

自描述二进制格式 MEMT，核心设计：

- **环形缓冲区 + generation**：固定数量 chunk 组成环形缓冲，chunk 回收时 generation+1，读者通过比较 generation 前后值检测并丢弃回收 chunk
- **单写者 + 无锁读者**：写者通过 `&mut self` 保证排他，读者通过 `Acquire`/`Release` 内存序完全无锁
- **三种后端统一抽象**：Heap（进程内）、Shm（POSIX `shm_open` 跨进程）、File（mmap 持久化）
- **per-chunk 字符串去重**：`DedupState` 在 chunk 范围内对重复字符串做哈希去重，存储为 4 字节回引用，可节省 20%+ 空间

这是整个项目中设计最精巧的组件之一。训练进程写入 mmap 文件，probing 服务器通过同一文件读取——零拷贝、零序列化。

### 2.5 Skills 诊断剧本系统

声明式诊断知识系统，每个 skill 由 `SKILL.md`（人类文档）+ `steps.yaml`（机器可执行）组成。`SkillBackend` trait 抽象了查询执行，使同一套 skill 可以在 CLI、Web WASM、MCP 三种环境中运行。

**亮点：**
- `resolve_use_global()` 自动检测 peer 数量决定是否使用 global 查询
- `cluster_integrity_findings()` 将不完整的 fanout 自动提升为 error 级 finding
- `ensure_read_only_sql()` 强制 skill 步骤只执行 SELECT 类查询

**不足：**
- `expand_template` 使用朴素 `str::replace`，缺乏转义机制（实际风险低但理论上存在）
- interpretation rules 的 `when` 语法是自定义 DSL，如 `step:xxx | column:yyy | max/min(ratio) > 1.5`，非标准表达式引擎

---

## 三、实现质量评价

### 3.1 出色的工程实现

**1. TorchProbe 采样引擎**（Python, 1554 行）

多重继承的 Mixin 架构 `TorchProbe(BaseTracer, Timer, Sampler, PythonTracer, VariableTracer)`，三个关键设计：

- **分层采样**：`_sample_period = round(1/rate)`，每周期恰好采样一个 step，均匀分布。使用 `blake2b` 哈希确保跨 rank 一致且不扰动宿主 RNG 流
- **Shadow Step 机制**：4:1 节奏（4 个 probed step + 1 个 baseline），shadow step 完全跳过 hook 仅记录墙钟时间，用于量化探测开销本身
- **延迟 GPU 事件读取**：采样步将 `DelayedRecord` 存入 `_deferred` 列表，至少等待 3 步（`_DEFER_MIN_SETTLE`）才尝试非阻塞 `event.query()`，最多延迟 16 步（`_DEFER_MAX_LAG`）后强制 synchronize。后台线程 `DeferredDrainWorker` 异步执行保存

**2. 错误处理链**

`EngineError` 采用 `thiserror`，通过 `#[from]` 自动转换保持完整因果链。明确拒绝 `From<String>` 防止"字符串化"错误类型。单一边界转换 `EngineError -> DataFusionError` 避免了每个调用点重复映射。生产路径禁止 `unwrap`/`expect`/`panic!`。

**3. NCCL Profiler 直接符号导出**

`nccl-profiler` 作为 cdylib 编译，直接导出 `ncclProfiler_v3` 和 `ncclProfiler_v4` C 符号。NCCL 运行时通过 dlopen 加载此符号，实现零侵入的 NCCL 操作采集。pool/pressure 机制避免了在高频采集路径上分配内存。

**4. Python 三层导入模式**

通过 `is_lightweight_module()` 和 `is_probing_cli()` 区分三种加载模式，解决了 `import probing` 副作用问题——很多场景下用户只需要检查版本，不希望启动整个 Rust 引擎。

**5. 跨平台 GPU 后端抽象**

GPU 扩展通过 `backend/registry.rs` 实现多后端：macOS（ioreg + sysctl）、NVIDIA（nvidia-smi）、统一 trait 接口。同时考虑了非 NVIDIA 生态（华为昇腾 HCCL shim）。

### 3.2 测试体系

两层测试结构：unit（快、隔离）和 regression（集成、契约、E2E）。

**亮点：**
- Rust 内嵌测试覆盖充分：`engine.rs` 780 行测试、`memtable.rs` 1,030 行测试（含 4 线程并发读写测试）
- `test_torch_probe_sampling.py`（815 行）覆盖了 settle window、force flush、RNG 不变性、inplace ReLU 兼容等边界条件
- `api_spec.json` 作为 HTTP API 契约 SSOT，`TOP_LEVEL_ROUTES` 常量与之对齐

**不足：**
- `test_engine.py` 仅 24 行，Python 核心层测试覆盖不足
- 缺少多进程 mock 的端到端联邦测试（主要通过进程内 `set_remote_query_hook` 模拟）
- `aggregate_pushdown.rs`（642 行复杂 SQL AST 操作）的测试覆盖度需验证

### 3.3 值得关注的技术债务

| # | 问题 | 严重度 | 说明 |
|---|------|--------|------|
| 1 | `probing-python` crate 过度膨胀 | 中 | 53K 行承载多个独立功能域，maturin 单 wheel 约束导致 |
| 2 | CPython 版本绑定维护负担 | 中 | 12 个版本（v2.7~v3.13）的内部结构偏移绑定，每次 CPython 更新需新增 |
| 3 | `memtable_sql.rs` 职责过重 | 中 | 2,125 行承载 mmap 发现、schema 推断、冷热分层、compaction 等 |
| 4 | 字符串匹配的错误处理 | 低 | `is_missing_table_error` 使用 `msg.contains("not found")` 判断 |
| 5 | `expand_template` 缺乏转义 | 低 | 朴素 `str::replace`，实际风险低但理论存在 |
| 6 | `ext/ray.py` 使用 `hash()` 生成 ID | 低 | Python hash 随机化导致不可复现 |
| 7 | 部分 `ext/` 异常处理过宽 | 低 | bare `except Exception: pass` 可能隐藏 bug |

---

## 四、架构决策评价

### 4.1 优秀决策

1. **DataFusion 而非自研 SQL 引擎** — 避免了重新发明 parser/optimizer/executor 的巨大成本，同时获得了标准 SQL 兼容性
2. **mmap 共享内存作为 IPC** — 三种后端统一了进程内/跨进程/持久化场景，零拷贝零序列化
3. **声明式 Skills 系统** — 诊断知识以 YAML+MD 形式存在，新增诊断不需要重新编译 Rust
4. **联邦三路径路由** — 基于查询语义选择最优执行路径，避免一刀切的 fan-out 开销
5. **MCP 协议集成** — 让 AI Agent 可以直接查询引擎、运行诊断 skill，写入操作受环境变量控制
6. **Rust 核心 + Python 薄包装** — 性能关键路径在 Rust，灵活性需求在 Python，PyO3 桥接

### 4.2 可改进方向

1. **`probing-python` 拆分** — 即使 maturin 约束存在，也可以通过 feature gate 在逻辑上拆分子模块
2. **结构化错误匹配** — 将 `is_missing_table_error` 等字符串匹配替换为 DataFusion 错误类型的结构化匹配
3. **模板引擎升级** — Skills 模板替换引入轻量模板引擎（如 `format!` 风格或 `minijinja`），支持转义
4. **端到端联邦测试** — 补充多进程 mock 的 E2E 联邦测试，而非仅依赖进程内 hook 模拟
5. **Python 测试覆盖** — 加强 `test_engine.py` 等核心 Python 模块的测试深度

---

## 五、总体评价

### 评分

| 维度 | 评分 | 说明 |
|------|------|------|
| 架构设计 | ★★★★★ | 四层分层清晰，契约边界明确，组合根集中 |
| 代码质量 | ★★★★☆ | Rust 侧优秀，Python 侧良好，少量技术债务 |
| 测试体系 | ★★★★☆ | Rust 内嵌测试充分，Python 侧有提升空间 |
| 文档质量 | ★★★★★ | modularity.md 等设计文档达到工程级别 |
| 创新性 | ★★★★★ | 联邦三路径、延迟GPU读取、Shadow Step 等设计新颖 |
| 可维护性 | ★★★★☆ | 技术债务有追踪，但 probing-python 膨胀是长期隐患 |

### 结论

Probing 是一个**架构成熟度很高**的分布式训练诊断系统。它成功地将 SQL 查询引擎、跨进程共享内存、联邦查询、AI Agent 集成、声明式诊断剧本等多个复杂组件组合成一个连贯的系统。Memtable 的无锁并发设计和联邦三路径路由是两个特别出色的工程实现。`modularity.md` 文档的存在表明团队有意识地管理模块边界和技术债务。

主要改进方向是 `probing-python` crate 的逻辑拆分、部分字符串匹配逻辑的结构化、以及 Python 侧测试覆盖的加强。整体而言，这是一个在系统设计层面超出大多数同类工具的项目——它不是在做"又一个 profiler"，而是在重新定义"agent-native 诊断"的交互范式。

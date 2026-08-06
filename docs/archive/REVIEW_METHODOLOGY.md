# Probing 方法论审视：从"工具实现"到"万卡可观测性方法论"

> **审视角度**：不以 probing 当前实现了什么为出发点，而以"万卡级训练可观测性需要什么方法论"为出发点，反视 probing 的实现完整度。
> **目的**：为博士实习生论文提供方法论框架，使叙事从"我们做了一个工具"升级为"我们提出了一套方法论，probing 是其实现验证"。

---

## 一、核心问题重新定义

### 1.1 传统范式的崩溃

万卡规模下，传统的 "profile-then-analyze" 范式在三个维度同时崩溃：

| 维度 | 小规模 (≤256 GPU) | 万卡 (10K+ GPU) | 崩溃原因 |
|------|-------------------|-----------------|---------|
| 数据量 | GB 级 trace，可收集 | TB 级 trace，收集本身成为瓶颈 | O(N) 数据 × O(N) rank = O(N²) 传输 |
| 故障复杂度 | 单点故障，简单排序可定位 | 级联故障，延迟多跳扩散 | ring/tree 拓扑中 1 个慢点 → 多跳传播 |
| 人工可行性 | 专家 30 分钟可分析 | 10K rank 无人能遍历 | 即使每 rank 1 秒，也需近 3 小时 |

### 1.2 方法论的核心主张

**在万卡规模下，训练可观测性必须从"收集-分析"范式转变为"查询-归因-闭环"范式。**

这个范式转换包含五个相互关联的原则，构成一个完整的方法论框架：

---

## 二、方法论框架：五大原则

### 原则 1：Query-Driven Observability（查询驱动可观测性）

**主张：** 不要先收集数据再分析，而要将查询推送到数据所在位置，只传输查询结果而非原始数据。

**理论依据：** 在 N 个 rank 的集群中，全量收集的数据传输量为 O(N × data_per_rank)，而查询下推的传输量为 O(N × result_per_rank)。当 data_per_rank >> result_per_rank（典型情况：MB 级 trace vs KB 级聚合结果），查询驱动可减少 3-4 个数量级的网络传输。

**方法论要求：**
- 联邦查询路由器，根据查询语义选择最优执行路径
- 聚合下推（SUM/COUNT/MIN/MAX 在各 rank 本地执行）
- 查询护栏（防止全表扫描导致网络风暴）
- **基于代价的自适应路由**（根据节点数、网络带宽、数据量动态选择路径）

### 原则 2：Execution-Model-Aware Attribution（执行模型感知归因）

**主张：** 不要依赖统计相关性推断故障原因，而要利用通信库的执行模型（NCCL proxy 线程语义、ring/tree 拓扑）进行确定性因果归因。

**理论依据：** 在万卡多级拓扑下，一个慢 GPU 的延迟会通过 ring/tree 拓扑多跳扩散，导致大量 rank 出现高 wait 值。统计相关性会将受害者误判为罪魁祸首。只有基于执行模型的因果推理才能正确区分"谁慢"和"谁因别人慢而等待"。

**方法论要求：**
- 通信操作的时间分解（proxy step wait decomposition）
- 异步完成语义的正确建模（引用计数完成模型）
- **通信拓扑图的构建与使用**（ring order、tree parent-child）
- **拓扑感知因果传播分析**（沿拓扑边追溯延迟源头）

### 原则 3：Calibrated Overhead Budgeting（校准式开销预算）

**主张：** 不要将 profiling 开销视为固定成本，而要主动测量、预测和控制自身开销，建立可验证的开销不变量。

**理论依据：** 在万卡规模下，1% 的 per-GPU overhead = 100 个 GPU 的算力浪费。未校准的开销在万卡规模下会导致显著的训练吞吐损失。必须建立开销模型，并证明其在规模扩展时保持有界。

**方法论要求：**
- Shadow 基线机制（A/B 对照测量探测开销）
- 延迟读取策略（减少同步操作）
- **集群级开销聚合模型**（N 个 rank 的 overhead 分布，而非单点）
- **前馈预测模型**（给定采样率预测开销）
- **闭环反馈控制**（测量 overhead → 自动调整采样率）
- 可验证的开销不变量（形式化定义 + 自动化测试守护）

### 原则 4：Declarative Diagnosis Knowledge（声明式诊断知识）

**主张：** 不要将诊断逻辑硬编码在程序中，而要将诊断过程表达为声明式、可组合、可复用的知识资产。

**理论依据：** 万卡训练的故障模式多样性超过了任何单一工具能硬编码的范围。诊断知识必须从工具实现中分离，使其可以被社区贡献、组合和演进。

**方法论要求：**
- 诊断过程的声明式表达（YAML/DSL）
- 人类可读的诊断文档（与机器可执行的定义共存）
- **诊断技能的可组合性**（一个 skill 的输出作为另一个 skill 的输入）
- 诊断结果的标准化表达（findings + severity + next_steps）
- 诊断知识库的版本管理和社区共享

### 原则 5：Agent-Native Interface（Agent 原生接口）

**主张：** 不要为人类 GUI 交互设计接口，而要为 AI Agent 的程序化交互设计接口。在万卡规模下，Agent 不是辅助工具而是唯一可行的诊断执行者。

**理论依据：** 10K rank 的诊断数据无法被人类有效遍历。AI Agent 可以在秒级完成跨 rank 聚合查询、多轮诊断推理、知识库匹配和修复建议生成。但这要求接口设计以 Agent 为一等公民，而非事后适配。

**方法论要求：**
- 结构化查询接口（SQL + schema discovery）
- 标准化 Agent 协议（MCP）
- 诊断技能的程序化调用
- 写操作安全控制
- 诊断上下文的跨轮次保持

---

## 三、Probing 实现完整度审计

### 3.1 审计矩阵

| 原则 | 方法论要求 | Probing 当前实现 | 完整度 | Gap 性质 |
|------|-----------|-----------------|--------|---------|
| **P1 查询驱动** | 基于代价的自适应路由 | 纯 AST 模式匹配 | 40% | 路由不考虑节点数/网络/数据量 |
| | 聚合下推 | SUM/COUNT/MIN/MAX 下推 | 90% | merge-safe 判定完整 |
| | 查询护栏 | LIMIT 注入 + broadcast 限制 | 80% | 缺少自适应 LIMIT |
| | 自适应路由 | 无 | 0% | 固定 fanout=128，不随规模调整 |
| **P2 执行模型归因** | Wait 分解 | SendGpuWait→PeerWait→Wait | 90% | 首次进入优先设计正确 |
| | 引用计数完成模型 | live_children + stopped | 90% | 正确处理 proxy progress loop |
| | 通信拓扑图 | 无 | 0% | peer/channel_id 采集但未构建拓扑 |
| | 拓扑感知因果传播 | wait 值独立排序 | 20% | 无 ring/tree 因果链推理 |
| **P3 校准开销** | Shadow 基线 | 4:1 cadence shadow step | 90% | 设计精巧 |
| | 延迟读取 | Deferred GPU Event Read | 90% | settle window + max lag |
| | 集群级开销模型 | 无 | 10% | 只有 per-rank 模型 |
| | 前馈预测 | 无 | 0% | 只有后验测量 |
| | 闭环反馈控制 | 无 | 0% | 静态配置，手动调整 |
| | 开销不变量 | 6 条形式化不变量 + 测试 | 80% | 但不随集群规模扩展 |
| **P4 声明式诊断** | 声明式表达 | YAML steps + MD 文档 | 80% | 13 个 skill |
| | 可组合性 | 无 | 10% | next_steps 是纯文本建议 |
| | 标准化结果 | findings + severity | 80% | 结构化输出 |
| | 知识库管理 | 无版本管理 | 20% | 文件系统存储 |
| **P5 Agent 原生** | 结构化查询 | DataFusion SQL + schema | 90% | 完整的表发现和查询 |
| | MCP 协议 | 8 个 MCP 工具 | 90% | 读操作完整 |
| | 程序化 skill 调用 | run_skill MCP 工具 | 80% | 但不支持 skill 链式调用 |
| | 写操作控制 | PROBING_MCP_ALLOW_WRITE | 90% | 安全控制完整 |
| | 跨轮次上下文 | 无 | 20% | Agent 需自行管理上下文 |

### 3.2 完整度雷达

```
        P1 查询驱动 (40%)
            |
   P5 Agent (70%) --- P2 执行模型归因 (50%)
            |              |
   P4 声明式 (50%) --- P3 校准开销 (45%)
```

**总体实现完整度：约 51%**

这个数字本身是论文的重要素材——它说明方法论的范围远超单一实现，probing 验证了方法论的核心可行性，但完整方法论的实施需要更多工作。

### 3.3 关键 Gap 详解

#### Gap 1：联邦路由无代价模型（P1）

**当前实现**（`route.rs` 第 44-56 行）：

```rust
pub fn classify_federated_sql(sql: &str) -> FederatedQueryPath {
    let lower = sql.to_lowercase();
    if !lower.contains("global.") { return Local; }
    if !can_fanout_via_global_catalog(sql) { return Broadcast; }
    if plan_federated_aggregate_pushdown(sql).is_some() {
        return AggregatePushdown;
    }
    FederatedScan
}
```

**问题**：路由决策完全基于 SQL 语法结构，不考虑：
- 节点数量（2 个节点和 2000 个节点走同一路径）
- 网络带宽/延迟（无成本估算）
- 数据量/选择性（不考虑 WHERE 过滤率）
- 节点负载（无运行时统计反馈）

`cluster_executor.rs` 中 fanout 并发度固定为 128，不随集群规模调整。

**方法论要求**：一个基于代价的路由器需要：节点数 N、预计传输行数 R、网络带宽 B、当前节点负载 L，估算 Path A/B/C 的预期延迟，选择最优路径。

**论文价值**：这是从"语法路由"到"语义+代价路由"的跃迁，有算法贡献空间。

#### Gap 2：归因算法无拓扑感知（P2）

**当前实现**（`nccl_culprit_victim/steps.yaml`）：

```sql
-- culprit: 谁的 send_gpu_wait 最高
SELECT rank, ... FROM {table}
ORDER BY send_gpu_wait_ns DESC LIMIT 10

-- victim: 谁的 recv_wait 最高
SELECT rank, ... FROM {table}
ORDER BY recv_wait_ns DESC LIMIT 10
```

**问题**：
- `peer` 列被采集但完全未用于归因——不知道"谁在等谁"
- `channel_id` 被采集但未用于重建 ring 结构
- 无因果链推理：不能推断"A 慢 → B 等 A → C 等 B"的传播链
- 在万卡多级拓扑下，简单 wait 排序会将级联受害者误判为 culprit

**方法论要求**：
1. 从 `peer` + `channel_id` + `is_send` 列构建通信拓扑图
2. 将 wait 值映射到拓扑边上
3. 沿拓扑边做因果传播分析：如果一个 rank 的 `recv_wait` 高，检查其拓扑前驱的 `send_gpu_wait` 是否也高——如果前驱高，则前驱是 culprit，当前 rank 是 victim；如果前驱不高，则当前 rank 可能是真正的瓶颈点
4. 处理 ring 和 tree 两种拓扑的不同传播模式

**论文价值**：拓扑感知因果归因是最核心的算法贡献，直接解决万卡场景的级联故障误判问题。

#### Gap 3：无自适应采样闭环（P3）

**当前实现**：采样率通过环境变量静态配置（`PROBING_TORCH_PROFILING=random:0.05`），运行时不会根据 overhead 测量自动调整。虽然 Shadow Step 机制测量了开销，但没有代码读取这些开销指标后回调 `set_sampling_mode()`。

**方法论要求**：闭环控制回路——
1. Shadow Step 持续测量 overhead
2. 当 overhead 超过阈值时，自动降低采样率
3. 当 overhead 低于阈值时，可以适当提高采样率以获取更精细数据
4. 在集群级别协调各 rank 的采样率（避免部分 rank 过载）

**论文价值**：这是一个控制系统设计贡献，可以形式化为闭环控制模型。

#### Gap 4：Skill 无可组合性（P4）

**当前实现**：`next_steps` 是 `Vec<String>` 纯文本建议，不会自动触发另一个 skill。一个 skill 的查询结果（DataFrame）不能作为另一个 skill 的输入参数。

**方法论要求**：
- Skill pipeline：`nccl_culprit_victim` 输出 culprit rank → 自动作为 `gpu_pressure` 的输入参数
- Skill DAG：定义 skill 间的依赖关系和执行顺序
- Skill 输出标准化：统一的结果格式，可被下游 skill 解析

**论文价值**：这是从"诊断脚本集合"到"诊断知识图"的跃迁。

---

## 四、对博士实习生论文的指导

### 4.1 当前问题诊断

博士生被局限在 probing 实现上的典型表现：

| 被局限的叙事 | 应该的叙事 |
|-------------|-----------|
| "我们实现了联邦 SQL 查询" | "我们提出查询驱动可观测性原则，联邦 SQL 是其实现方式之一" |
| "我们做了 NCCL wait 分解" | "我们提出执行模型感知归因原则，wait 分解是其核心机制" |
| "我们用了 Shadow Step" | "我们提出校准式开销预算原则，Shadow Step 是其测量机制" |
| "我们集成了 MCP" | "我们提出 Agent-Native 诊断原则，MCP 是其接口实现" |
| "probing 能做 X" | "方法论要求做 X，probing 实现了 X 的 Y%" |

### 4.2 建议的论文叙事框架

```
Title: Observability at Scale: A Methodology for
       10,000-GPU Training Diagnosis

Abstract:
  At 10,000+ GPU scale, the traditional "profile-then-analyze"
  paradigm collapses: data volumes exceed collection capacity,
  cascading failures defy statistical attribution, and human
  traversal becomes infeasible. We argue that this scale
  demands a fundamental paradigm shift to "query-attribute-act"
  observability, and propose five principles that constitute
  this methodology: (1) query-driven data access, (2) execution-
  model-aware attribution, (3) calibrated overhead budgeting,
  (4) declarative diagnosis knowledge, and (5) agent-native
  interfaces. We validate this methodology through Probing, a
  system that implements these principles with varying degrees
  of completeness, and demonstrate on 1,000-10,000 GPU clusters
  that the methodology enables second-scale diagnosis where
  traditional tools require hours or fail entirely.

Contributions:
  1. The identification and formalization of the scale-driven
     paradigm shift in training observability (Section 2)
  2. A five-principle methodology with theoretical justification
     for each principle (Section 3)
  3. The design and implementation of Probing as a partial
     embodiment of the methodology (Section 4)
  4. Large-scale evaluation demonstrating the methodology's
     effectiveness, including identification of gaps between
     methodology and implementation as future work (Section 5)
```

### 4.3 关键叙事策略

**策略 1：Gap 是贡献，不是缺陷**

论文不应该回避 probing 实现的 gap，而应该将 gap 作为方法论价值的证明——"方法论的范围远超单一实现，以下是当前实现完整度的审计，以及每个 gap 的理论影响"。

具体做法：在 Evaluation 后加一节 "Methodology Coverage Analysis"，用 3.1 节的审计矩阵展示 probing 对五原则的实现完整度（约 51%），并讨论每个 gap 的影响和未来方向。

**策略 2：万卡实验验证方法论，不只是验证工具**

实验设计应回答的问题是"方法论的原则在万卡规模下是否必要且充分"，而不是"probing 在万卡规模下是否工作"。

具体做法：
- P1 验证：在 10K 节点上展示查询下推 vs 全量收集的传输量差异（验证"查询驱动"原则的必要性）
- P2 验证：在多级拓扑下展示简单 wait 排序 vs 拓扑感知归因的准确率差异（验证"执行模型归因"原则的必要性）——这需要先实现拓扑感知归因
- P3 验证：在 10K 节点上展示 overhead 是否保持有界（验证"校准开销"原则的有效性）
- P5 验证：在 10K 节点上展示 Agent 自主诊断 vs 人工诊断的可行性差异（验证"Agent-Native"原则的必要性）

**策略 3：方法论驱动的新贡献**

利用方法论框架，可以推导出 probing 当前未实现但方法论要求的贡献点。这些新贡献既是论文的算法/系统贡献，也是方法论价值的验证：

| 方法论要求 | 需要实现的新贡献 | 论文中的位置 |
|-----------|----------------|-------------|
| P1 代价路由 | 基于节点数/带宽/数据量的自适应路由器 | Section 3.1 + Evaluation |
| P2 拓扑归因 | 通信拓扑图构建 + 因果传播分析算法 | Section 3.2 + Evaluation |
| P3 闭环控制 | overhead → sampling rate 反馈控制器 | Section 3.3 + Evaluation |
| P4 Skill 组合 | Skill DAG + 输出-输入管道 | Section 3.4 (design) + Discussion |

**策略 4：Related Work 从方法论角度分类**

不要按工具分类（PyTorch Profiler、Nsight、HTA...），而按方法论原则分类：

```
Related Work:
  Query-driven observability: DeepFlow (eBPF + SQL), ...
  Execution-model attribution: Cascon (congestion attribution), ...
  Calibrated overhead: ...
  Declarative diagnosis: ...
  Agent-native systems: ...

  No existing system combines all five principles.
```

### 4.4 最优先实现的 Gap

如果时间有限，建议优先实现以下两个 gap——它们既是方法论的核心验证，又有最高的论文 ROI：

**优先级 1：拓扑感知归因算法（P2 Gap）**

理由：
- 这是方法论中最核心的算法贡献
- 万卡场景的级联故障问题只有拓扑感知归因能解决
- probing 已经采集了 `peer`、`channel_id`、`is_send` 数据，只差分析逻辑
- 可以直接与当前简单排序做 ablation 对比

实现路径：
1. 从 NCCL profiler 采集的 `peer` + `channel_id` + `is_send` 列构建通信图
2. 对每个 collective 操作，重建 ring/tree 拓扑
3. 在拓扑图上做 wait 值的因果传播分析
4. 输出：每个高 wait rank 的"根本原因 rank" + 传播路径

**优先级 2：自适应联邦路由（P1 Gap）**

理由：
- 联邦查询是 probing 的标志性能力，当前的路由器太简单
- 万卡场景下，固定 fanout=128 可能导致查询超时或网络拥塞
- 代价模型可以形式化，有理论贡献空间

实现路径：
1. 在 `classify_federated_sql` 中引入节点数 N 参数
2. 估算各路径的预期延迟：Path A = O(N × local_query + merge)，Path C = O(N² × data_transfer)
3. 根据预期延迟选择最优路径
4. 动态调整 fanout 并发度

---

## 五、修正后的论文结构

```
1. Introduction (1.5 页)
   - 万卡训练时代的可观测性挑战
   - 传统范式的崩溃（数据爆炸 + 级联故障 + 人工不可行）
   - 我们的方法论：五原则
   - Probing 作为方法论的实现验证
   - 贡献总结

2. Motivation & Background (2 页)
   2.1 万卡训练的通信模式
   2.2 传统工具在万卡规模的崩溃（实测数据）
   2.3 NCCL proxy 线程执行模型
   2.4 范式转换的必要性论证

3. Methodology (3 页) — 论文核心
   3.1 Query-Driven Observability
       - 原则定义 + 理论依据
       - 联邦查询路由算法（含代价模型）
   3.2 Execution-Model-Aware Attribution
       - 原则定义 + 理论依据
       - Wait 分解 + 拓扑感知因果归因算法
   3.3 Calibrated Overhead Budgeting
       - 原则定义 + 理论依据
       - Shadow Step + 闭环控制模型
   3.4 Declarative Diagnosis Knowledge
       - 原则定义 + 理论依据
       - Skill 表达 + 可组合性设计
   3.5 Agent-Native Interface
       - 原则定义 + 理论依据

4. Implementation (2 页)
   - Probing 架构概述（四层 + 契约）
   - 各原则的实现完整度（坦诚说明 gap）

5. Evaluation (4 页)
   5.1 实验环境 (1K / 10K GPU)
   5.2 范式对比: query-driven vs collect-then-analyze
   5.3 归因准确率: 拓扑感知 vs 简单排序 (故障注入)
   5.4 开销 scaling: per-GPU overhead vs cluster size
   5.5 Agent 自主诊断 vs 人工专家
   5.6 联邦路由: 代价路由 vs 固定路由
   5.7 Case Studies (2-3 个真实万卡故障)
   5.8 Methodology Coverage Analysis (实现完整度审计)

6. Discussion (0.5 页)
   - 方法论 vs 实现的 gap
   - 未实现原则的影响
   - 非 NVIDIA 生态扩展

7. Related Work (1 页)
   - 按方法论原则分类，非按工具分类

8. Conclusion (0.5 页)
```

---

## 六、总结

Probing 当前的实现完整度约为方法论的 51%。这不是问题——这恰恰是论文的价值所在。

**方法论是贡献，实现是验证。** 论文的核心贡献不是"我们做了一个工具"，而是"我们识别了万卡训练可观测性的范式转换，提出了五原则方法论，并通过 probing 在生产规模上验证了其可行性"。

博士生需要做的思维转换：

1. **从"probing 能做什么"到"方法论要求做什么"** — probing 的实现只是方法论的一个实例化
2. **从"实现细节"到"设计原则"** — 论文评审关心的是原则，不是代码
3. **从"功能列表"到"完整度审计"** — gap 是贡献的一部分，不是需要隐藏的缺陷
4. **从"工具对比"到"方法论对比"** — 相关工作应按原则分类，而非按工具分类

**最关键的一句话**：不要让 probing 的当前实现定义论文的边界，而要让方法论定义 probing 的未来。

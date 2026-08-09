# 分布式 Profiler 查询与可视化

> 状态：架构设计。当前已经具备短窗口 Torch Profiler 采集、
> `python.profile_capture` / `python.profile_hotspot` 和基础联邦查询；本文定义的是面向
> 万 Rank Timeline、火焰图和跨 Rank 分析的统一模型，其中 `timeline.*` 尚未全部实现。
>
> 相关基础：[性能分析](profiling.zh.md) · [联邦查询](federation.zh.md) ·
> [分布式成员](distributed.zh.md)

## 1. 目标与整体架构

一次短窗口如果每个 Rank 产生 50 万个 Event，一万个 Rank 就有 50 亿行。把所有 Event
集中上传再查询，会让网络、协调器内存、排序时间和浏览器渲染量都随原始数据规模增长。

Probing 的目标不是减少参与分析的 Rank，而是减少跨层传输的数据：

> 所有目标 Rank 都参与计算，但只交换当前问题、时间窗口和显示分辨率需要的结果。

![分布式 Profiler 查询整体架构](../assets/profiler-distributed-query.svg)

架构必须同时满足四个目标：

| 目标 | 含义 |
|------|------|
| 全量参与 | 一次查询可以覆盖全部目标 Rank，并显式报告缺失成员 |
| 本地收敛 | Filter、区间合并、Top-K、分位数状态尽量在数据所在位置计算 |
| 连续下钻 | 从作业概览进入节点、行为组、异常 Rank，最终读取精确 Event |
| 证据一致 | SQL、可视化和 Agent 使用同一时间窗口、Rank 集合与质量信息 |

系统不尝试在首屏返回一万个完整 Timeline，也不允许把各 Rank 独立执行的复杂 SQL 结果简单
拼接后冒充全局 JOIN、窗口函数或分位数结果。

## 2. 统一数据模型

数据模型由 Timeline、Tile 和 Flamegraph 三组虚拟实体组成。它们是查询语义，不要求采用某种
固定物理存储：实现可以读取 Trace 分区、MEMT/MEMC、预计算摘要或外部 Profiler 文件。

### 2.1 Timeline 实体

| 虚拟表 | 一行表示什么 | 关键关系 |
|--------|--------------|----------|
| `timeline.capture` | 某 Rank 参与一次分布式采集的状态 | `capture_group_id` 连接全作业 |
| `timeline.track` | CPU 线程、GPU Stream、逻辑轨道或 Counter 轨道 | `parent_track_id` 形成层级 |
| `timeline.slice` | Step、Op、Kernel、Memcpy、Collective 或 Wait 区间 | parent、correlation、stack |
| `timeline.flow` | Launch、同步、等待、Collective peer 等因果边 | source/target slice |
| `timeline.counter` | 某时刻的 GPU、NIC、CPU 或内存数值 | track + timestamp |

`timeline.capture` 是所有跨 Rank 查询的入口：

```text
capture_group_id, capture_id, run_id
rank, world_size, role, host, node_rank, local_rank
profiler_type, activities, step_begin, step_end
started_at_ns, ended_at_ns
status, events_total, events_dropped, truncated
clock_domain, clock_error_ns, error
```

“Rank 没有 Capture”和“Capture 中没有匹配 Event”必须是两种状态。查询热点或异常之前，先由
Capture Manifest 确定 `ranks_expected`、`ranks_seen`、丢失事件和时钟质量。

`timeline.track` 与 `timeline.slice` 共同表达可查询时间线：

```text
track:
  capture_id, track_id, parent_track_id
  track_kind, process_id, thread_id, device_uuid, stream_id, name

slice:
  capture_id, slice_id, parent_slice_id, track_id, rank
  kind, name, normalized_name
  start_ns, duration_ns, global_step, microbatch_id
  correlation_id, operation_id, communicator_id, collective_seq
  stack_id, bytes, attributes
```

Slice 保存活动区间；Flow 保存跨轨道或跨 Rank 的关系。CPU→GPU Launch、同步等待和 Collective
成员关系不能只靠时间戳猜测，应优先使用 correlation、communicator 和 sequence 等逻辑键，并
在只能推断时返回 `method` 与 `confidence`。

### 2.2 多分辨率 Tile

`timeline.tile` 是面向作业概览的结果，不是另一份原始 Trace：

```text
capture_group_id, resolution_level
rank_group_kind, rank_group_id, rank_set_token
ranks_total, ranks_seen
bucket_start_ns, bucket_end_ns
category
occupancy_p50, occupancy_p95, occupancy_max
duration_p50_ns, duration_p95_ns
event_count, outlier_count, dominant_operation
exact, error_bound, coverage
```

![时间与 Rank 双轴的多分辨率 Timeline](../assets/profiler-timeline-pyramid.svg)

Tile 同时压缩两个方向：时间轴按照像素宽度分桶，Rank 轴按照全作业、节点、并行角色、行为组
或具体 Rank 分组。用户放大窗口或收窄 Rank 集合后，查询自动选择更细 Tile，最终才返回精确
Slice。

时间占用不能直接对 Event Duration 求和。每个 Rank 必须先计算区间并集：

```text
occupancy(rank, bucket)
  = union(matching_intervals ∩ bucket) / bucket_width
```

然后才在 Rank 方向计算 P50、P95、Max 和异常数。这样重叠 Stream 不会被重复计时，典型 Rank
和尾部 Rank 也不会被一个平均数掩盖。

### 2.3 分布式火焰图

火焰图节点需要保存路径、消耗分布和 Rank 覆盖，而不是附加一万个 Rank ID：

```text
path_id, parent_path_id, frame_name, frame_kind, depth
metric, inclusive_value, self_value
rank_count, rank_coverage, rank_set_token
value_p50, value_p95, value_max, outlier_count
subject_value, reference_value, delta_value, delta_ratio
exact, error_bound, coverage
```

Rank 集合以压缩 Bitmap 或服务端 Token 表达。调用路径通过稳定 Path Hash 在 Rank、Node 和
Coordinator 逐级合并。

Torch Profiler 适配器仍可保留 `profile_capture` 和 `profile_hotspot` 作为 Capture 与热点摘要
视图；完整 Kineto Event 通过相同语义投影为 Track、Slice、Flow，而不是要求日常 SQL 扫描
Chrome `traceEvents`。

## 3. 分布式查询与执行

Web 和 Agent 通过类型化 `TimelineQuery` 表达问题，SQL 用于查询虚拟表和结构化结果。客户端
不需要自己拼接万 Rank Fan-out、分位数合并或区间运算 SQL。

### 3.1 查询合同

```yaml
scope:
  capture_group_id: group-42
  steps: {from: 1000, to: 1020}
  ranks: all
alignment: {kind: global_step, anchor: step_begin}
tracks:
  group_by: [node, behavior_cluster]
  include: [step, phase, gpu, collective]
events:
  kinds: [cpu_op, gpu_kernel, collective, synchronization]
reduce:
  time: interval_occupancy
  ranks: [p50, p95, max, outlier_count]
resolution:
  width_pixels: 1600
  max_rows: 200
  detail: auto
output: {kind: timeline_tiles}
```

对齐方式必须显式选择：

| 对齐 | 用途 | 边界 |
|------|------|------|
| `wall_clock` | 已校准机器之间的绝对因果 | 依赖时钟误差 |
| `global_step` | 比较同一训练 Step 的结构和时长 | 不能证明跨机绝对先后 |
| `collective` | 分析到达偏斜和传输阶段 | 依赖 communicator/sequence |
| `operation` | 比较同名第 N 次 Op/Kernel | 依赖规范化名称和匹配规则 |
| `custom_marker` | 用户定义阶段 | 依赖 Marker 覆盖率 |

Rank 使用结构化选择器：all、node、role、behavior group、outliers 或少量显式 Rank。大集合由
`rank_set_token` 在后续下钻中复用，避免客户端反复传输巨大整数数组。

### 3.2 三段执行计划

![Rank、Node 与 Coordinator 三段执行](../assets/profiler-timeline-execution.svg)

| 计划 | 执行内容 | 输出 |
|------|----------|------|
| Rank | Filter、时间裁剪、区间并集、局部 Top-K、Folded Stack | 有界 Partial State |
| Node | 合并本机 Rank，关联 GPU/NIC/PCIe/NUMA，生成节点摘要 | Node Partial |
| Coordinator | 全局分位数、异常 Rank、行为组、视图与质量收据 | 查询结果 |

网络交换的是可归并状态，而不是默认交换原始 Event：

| 目标 | 交换状态 |
|------|----------|
| Duration 分布 | KLL / t-digest 等 Quantile Sketch |
| 热点 | 有界 Top-K State |
| Rank 覆盖 | 压缩 Bitmap / Rank Set Token |
| 时间占用 | 每 Rank 的 Interval Occupancy |
| 火焰图 | Path Hash + Metric + Distribution Sketch |
| Collective | communicator/sequence + entry/ready/complete |

精确 Slice 只在少量 Rank 和窄时间窗下交换。分层网络拓扑复用
[联邦查询的 Coordinator→local0→leaf 模型](federation.zh.md#hierarchical-fan-out)，但 Timeline
计划必须声明每一层的归并函数，不能退化成广播任意 SQL 后拼接。

### 3.3 流式结果与质量

Arrow RecordBatch 按 Partition 流式返回；用户改变视口或取消查询时，取消信号向 Node 和 Rank
传播。每个结果都携带统一收据：

```text
query_id, capture_group_id, membership_epoch
ranks_expected, ranks_seen, nodes_expected, nodes_seen
partitions_scanned, failed_partitions
rows_scanned, bytes_scanned
resolution_level, exact, error_bound, partial, elapsed_ms
```

概览数据量由 `width_pixels`、`max_rows` 和查询预算决定，而不是由原始 Event 数量决定。

## 4. 跨 Rank 可视化

一万个 Rank 的首屏不是“一 Rank 一条完整 Timeline”，而是多种共享上下文的问题视图。

![跨 Rank Timeline、热力图、瀑布图与拓扑视图](../assets/profiler-cross-rank-visuals.svg)

### 4.1 共享上下文

```yaml
capture_group_id: group-42
alignment: {kind: global_step, step: 1024}
time_window: {start_ns: -2000000, end_ns: 12000000}
rank_set_token: ranks-outlier-7
cohorts:
  subject: slow-group
  reference: healthy-group
filters: {event_kinds: [gpu_kernel, collective]}
```

时间窗、Rank 集合、Subject/Reference Cohort 和过滤器是所有视图的公共状态。用户在一个视图
框选后，其他视图重新查询同一上下文；Agent 也接收同一个 Context，而不是重新猜测用户看到
了哪些 Rank 和时间。

### 4.2 视图与问题

| 视图 | 表达方式 | 主要回答的问题 |
|------|----------|----------------|
| Rank × Time 热力图 | 行是节点/行为组/Rank，列是时间桶 | 异常发生在哪些 Rank 和时段 |
| 分位数 Timeline | P50/P95/Max 带状曲线 | 尾部从什么时候偏离多数 Rank |
| Collective 瀑布图 | entry→ready→transfer→complete | 是晚到还是传输变慢 |
| 行为组与代表 Timeline | 按 Signature 聚类并展示代表 Rank | 一万 Rank 中有几种执行模式 |
| Operation × Rank 热力图 | Op/Kernel 相对 Peer Baseline 的差值 | 具体哪个 Operation 产生偏斜 |
| 拓扑视图 | Node/NUMA/PCIe/NIC/Rail 空间布局 | 异常是否集中在硬件拓扑 |

行为 Signature 可由 Step Wall Time、Compute/Communication/Idle Occupancy、Top Operation Ratio
和 Collective 特征组成。聚类输出必须保留组内方差、拓扑分布、代表 Rank 和 Rank Set Token，
不能只返回一个不可解释的 Cluster ID。

Collective 瀑布图至少区分：前序计算结束、进入 Collective、成员 Ready、传输和完成。少数
Rank 晚到与所有 Rank 同时到达但 Transfer 变慢是两类不同问题，不能只用 Collective 总时长
着色。

### 4.3 火焰图模式

| 模式 | 宽度 | 颜色 | 用途 |
|------|------|------|------|
| 聚合 | 总量或典型值 | Frame 类型 | 找整体热点 |
| 差分 | `abs(subject-reference)` | Subject 增加/减少 | 慢组多花时间在哪里 |
| 方差 | 典型消耗 | 离散度或异常比例 | 哪条路径跨 Rank 不一致 |
| 覆盖 | 调用路径权重 | Rank Coverage | 控制流是否只出现在部分 Rank |

选择一个 Frame 后，通过 `rank_set_token` 打开贡献 Rank 的热力图，再进入代表 Rank 的精确
Timeline。Timeline、火焰图和瀑布图必须能沿同一证据链往返，而不是三个独立页面。

## 5. 正确性与资源边界

| 设计决策 | 必须遵守的边界 |
|----------|----------------|
| 本地计算、全局归并 | 每个跨 Rank 运算声明可归并状态和 Coordinator 函数 |
| 多分辨率优先 | 首屏只返回 Tile/Sketch；精确 Event 仅用于受限下钻 |
| 区间语义 | 重叠活动先做区间并集，禁止直接累加 Duration 当作 Wall Time |
| 显式对齐 | 每个结果记录 Clock/Step/Collective/Operation Anchor 与误差 |
| 显式覆盖率 | Missing Capture、No Matching Event、Dropped Event、Failed Partition 分开表达 |
| 有界资源 | Rank 数、时间范围、像素、行数、字节数和执行时间都进入查询预算 |
| 可取消 | 视口变化后旧查询必须停止 Rank/Node 侧工作 |
| 可追溯 | 可视化结论必须能回到 Query、Rank Set、时间窗、Slice 或 Flamegraph Path |

系统允许在概览阶段使用 Sketch 和近似聚合，但必须返回 `exact`、`error_bound`、`coverage` 和
`partial`。近似结果不能伪装成精确值，部分失败也不能通过空结果隐藏。

不支持的全局关系运算必须显式失败或进入专门的 Coordinator Plan：

- 不把 `global.a JOIN global.b` 在每个 Rank 独立执行后直接拼接成“全局 JOIN”；
- 不对每个 Rank 分别 `LIMIT K` 后声称得到全局 Top-K；
- 不把不可归并的 `count(distinct)`、窗口函数或重叠 Duration 当作普通 Sum；
- 不为一万个 Rank 生成单个 Chrome Trace JSON；完整导出使用 Manifest + 分区文件，选中少量
  Rank 后再生成 Perfetto/Chrome Trace。

最终输出保持四类：多分辨率 Timeline Tile、受限的精确 Slice、带 Rank 分布的 Flamegraph
Tree，以及供 SQL/Skill/Agent 使用的结构化分析结果。四类输出共享同一查询收据和证据坐标。

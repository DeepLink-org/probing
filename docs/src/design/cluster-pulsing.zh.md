# 基于 Pulsing 的集群管理

本文档描述如何让 probing 的集群管理复用 Pulsing 的分布式能力（gossip 成员、故障检测、命名发现），在保持现有 API 与业务语义的前提下，获得自动成员发现与存活检测。

## 现状

### probing 当前集群模型

- **数据源**：内存中的 `probing_proto::Cluster`，key 为 `host:addr`，带 `rank_index`（rank → 节点）。
- **Node 字段**（proto）：`host`, `addr`, `local_rank`, `rank`, `world_size`, `group_rank`, `role_name`, `role_rank`, `role_world_size`, `status`, `timestamp`。
- **写入路径**：
  - **rank 0**：本机直接调用 `update_node(node)`（见 `report.rs`）。
  - **其他 rank**：通过 HTTP `PUT /apis/nodes` 向「中心」（report_addr，通常为 rank 0 或独立 server）上报。
- **读取路径**：HTTP `GET /apis/nodes`、Web 集群页、extensions/cc 的 `nodes` 表（Arrow）。
- **特点**：无内置故障检测；依赖应用侧定时上报；中心聚合视图。

### Pulsing 提供的集群能力

- **Gossip 成员**：SWIM 风格，`GossipCluster`，周期 gossip + 故障检测（PFail → Fail）。
- **成员信息**：`MemberInfo`：`node_id`（u128）、`addr`（SocketAddr）、`status`（Alive/Suspect/Dead）、`incarnation` 等。
- **API**：
  - Rust：`cluster.all_members()` / `alive_members()`。
  - Python：`await system.members()` → `list[dict]`（`node_id`, `addr`, `status` 等）。
  - HTTP：`GET /cluster/members` 返回 JSON 成员列表。
- **发现**：seed 加入、named actor 注册、跨节点解析。

## 设计目标

1. **成员发现与存活**：由 Pulsing 负责「谁在集群里、谁还活着」，probing 不再仅依赖应用上报来推断存活。
2. **保留业务语义**：rank、world_size、role_name 等训练/作业语义仍由 probing 侧维护（上报或配置）。
3. **接口兼容**：现有 `get_nodes` / `put_node`、`nodes` 表、Web 集群页行为保持可用；可演进为「Pulsing 成员 + 业务元数据合并」视图。
4. **松耦合**：probing 自管、主动发现 Pulsing；不要求 Pulsing 或应用先「接好」probing。

## 松耦合原则

1. **probing 被注入后，自己管好自己**
   不依赖外部先启动或配置 Pulsing；probing 进程内行为自洽（上报、本地 CLUSTER、HTTP API 等照常工作）。

2. **cluster 模块主动发现 Pulsing**
   probing 的 cluster 模块在适当时机（例如 server 启动或首次访问集群视图时）**尝试发现**当前环境里是否已有 Pulsing（例如检测全局 ActorSystem、环境变量、或指定 URL/端口）。

3. **发现 Pulsing 后的两种路径**
   - **a. Pulsing 已初始化**
     若发现 Pulsing 的 ActorSystem 已经存在（例如用户代码已 `pul.init()`）：
     - 在该 ActorSystem 上**注册一个专用于集群管理的 actor**（例如 named actor `"probing/cluster"`）；
     - 节点发现通过**向该 actor 请求**或**该 actor 订阅/拉取 members()** 得到，再写回 probing 的 `CLUSTER`。
   - **b. Pulsing 未初始化，且配置了初始化方式**
     若未发现已初始化的 Pulsing，但配置中指定了如何初始化（例如 `pulsing_seeds`、standalone 等）：
     - probing **尝试自己初始化** Pulsing（例如在 Python 侧调用 `pul.init(seeds=...)` 或等价逻辑）；
     - 初始化成功后，再走路径 a：注册集群管理 actor，通过该 actor 做节点发现。

未发现 Pulsing 或未配置初始化时，cluster 模块**仅使用现有上报**构建视图，行为与当前一致。

## 集成形态（在松耦合下的表现）

- **有 Pulsing（已初始化）**：cluster 模块发现现有 ActorSystem → 通过拉取 members() 与上报合并写入 CLUSTER。
- **有 Pulsing（需由 probing 初始化）**：cluster 模块按配置初始化 Pulsing → 同上。
- **无 Pulsing**：不注册 actor，仅靠 PUT /apis/nodes 与 rank 0 本地 update_node 构建视图。

### 组网由 Pulsing 后台负责，调用方只调 API 等待

Pulsing 提供 **bootstrap** 模块：在**后台线程**中自动尝试组网（先 Ray 再 torchrun），对外只暴露 `wait_ready(timeout)` / `await_ready(timeout)`。probing 或其他调用方**不**实现 init_in_ray / init_in_torchrun，只需在需要集群时调用 `pulsing.bootstrap.wait_ready(timeout)` 等待，返回 True 后即可 `get_system()` 并使用。组网逻辑（谁当 seed、如何广播）全部在 Pulsing 内部完成。

### 借助 Ray / torchrun 组网（由 bootstrap 内部调用）

- **Ray**：`pulsing.integrations.ray.init_in_ray()`，首个进程通过 Ray KV 成为 seed，其余以 `seeds=[seed_addr]` 加入。
- **torchrun**：`pulsing.integrations.torchrun.init_in_torchrun()`，rank0 广播地址，其他 rank 以 seed 加入。

bootstrap 后台会按顺序尝试上述两种方式；probing 只需在启动同步前调用 `pulsing.bootstrap.wait_ready(PROBING_PULSING_BOOTSTRAP_TIMEOUT)` 等待即可。

## 数据流（松耦合）

```text
┌─────────────────────────────────────────────────────────────────┐
│  probing 进程（被注入后自管）                                      │
│  • cluster 模块尝试发现 Pulsing                                   │
└────────────────────────────┬──────────────────────────────────┘
                               │
           ┌───────────────────┼───────────────────┐
           ▼                   ▼                   ▼
    未发现 Pulsing     发现已初始化 Pulsing    未初始化但配置了 init
           │                   │                   │
           ▼                   ▼                   ▼
    仅用上报构建视图    在 ActorSystem 上       probing 自己 init
    (当前行为)          注册集群管理 actor      Pulsing，再注册 actor
                               │                   │
                               └─────────┬─────────┘
                                         ▼
┌─────────────────────────────────────────────────────────────────┐
│  Pulsing ActorSystem                                             │
│  • 集群管理 actor（如 "probing/cluster"）拉取 members()           │
│  • 映射为 Node 基础信息，与 PUT 上报的 rank/role 合并               │
└────────────────────────────┬──────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│  probing_core::cluster::CLUSTER                                 │
│  Cluster { nodes: host:addr -> Node, rank_index }                │
│  Node = 基础(Pulsing) + 业务(rank, world_size, role, ...)         │
└────────────────────────────┬──────────────────────────────────┘
                              │
              ┌───────────────┼───────────────┐
              ▼               ▼               ▼
        GET /apis/nodes   Web 集群页    extensions/cc nodes 表
```

- **合并规则**：以 `host:addr` 或 Pulsing `node_id` 为键；若某节点仅有 Pulsing 信息则只填基础字段，rank/role 为空或默认；若仅有上报则仅填业务字段，存活状态可标为「未知」或依赖超时剔除。
- **故障检测**：从 Pulsing 来的节点若 `status != Alive`，可在 probing 视图中标记为 Unhealthy/Unreachable 或从列表中过滤/标注。

## 接口与兼容性

- **保持**：`GET /apis/nodes`、`PUT /apis/nodes`、`cluster::get_nodes()`、`cluster::update_node()`、extensions/cc `nodes` 表 schema（host, addr, rank, world_size, role_*, status, timestamp）。
- **扩展**（可选）：
  - Node 或 API 增加 `pulsing_node_id`、`pulsing_status`，便于调试与展示。
  - 配置项：发现方式（如 `pulsing_auto_discover`）、初始化方式（如 `pulsing_seeds`、`pulsing_init_mode`）。
- **兼容**：未发现 Pulsing 且未配置初始化时，行为与当前一致，仅靠上报构建集群视图。

## 实施步骤建议

1. **Phase 1（发现 + 使用已有 Pulsing）**
   - cluster 模块实现「发现 Pulsing」：检测全局 ActorSystem（Python 侧 `pul.get_system()` / 是否已 init）、或环境变量/配置中的 Pulsing 入口。
   - 若已初始化：在 ActorSystem 上注册集群管理 actor（named，如 `"probing/cluster"`），该 actor 负责拉取 `members()`、映射为 `Vec<Node>`，并可由 probing 侧定时请求或订阅更新；合并进 CLUSTER。
   - 配置：仅「是否启用发现」与「集群管理 actor 名称」等，不强制要求配置 Pulsing。

2. **Phase 2（可选由 probing 初始化 Pulsing）**
   - 当未发现已初始化的 Pulsing 且配置了初始化方式（如 `pulsing_seeds`、standalone）时，cluster 模块（或 Python 桥）尝试调用 `pul.init(...)`（或等价）。
   - 初始化成功后走 Phase 1：注册集群管理 actor，通过该 actor 做节点发现。
   - 统一「成员源」：仅上报 / Pulsing（通过集群管理 actor）/ 合并。

3. **Phase 3（可选增强）**
   - 集群管理 actor 可对外提供「注册 rank/role」接口，与 PUT /apis/nodes 互补或替代。
   - 分布式存储（TopologyView）的节点列表也可从 Pulsing 成员驱动。

## 依赖与约束

- **依赖**：若在 Rust 侧直接依赖 Pulsing，需在 probing 的 Cargo 中增加 `pulsing-actor`（或仅通过 HTTP/FFI 调用则无 Rust 依赖）。
- **进程模型**：当前 report 仅 rank 0 写本地、其余 PUT 到中心；与 Pulsing 集成后，中心或 rank 0 需能访问 Pulsing 视图（同进程或远程）。
- **身份对应**：需要约定 Pulsing 节点与 probing 节点的对应关系（例如同一进程既上报 `host:addr` 又加入 Pulsing，则用 `addr` 或 node_id 关联）。

## 小结

- **松耦合**：probing 被注入后自管；cluster 模块主动发现 Pulsing；发现后要么用已有 ActorSystem 并注册集群管理 actor，要么在配置允许时由 probing 自己初始化 Pulsing 再注册 actor。
- **用 Pulsing 做**：通过「集群管理 actor」做成员发现与存活检测；该 actor 挂在 Pulsing 的 ActorSystem 上，由 probing 注册与使用。
- **probing 保留**：rank/world_size/role 等业务语义、现有 HTTP API、`nodes` 表与 Web 集群页；无 Pulsing 时行为不变。
- **集成方式**：发现 → 已初始化则注册 actor / 未初始化且配置则先 init 再注册 actor → 通过该 actor 拉取 members 并与上报合并写入 CLUSTER。

这样在不破坏现有使用方式、且不要求应用或 Pulsing 先接好 probing 的前提下，为 probing 带来基于 Pulsing 的自动集群发现与故障感知能力。

---

## 设计点评

### 优点

- **松耦合方向正确**：probing 自管、主动发现 Pulsing，无 Pulsing 时退化为纯上报，不绑架部署顺序，对现有用户友好。
- **职责清晰**：Pulsing 管「谁在、谁活」，probing 管 rank/role 等业务语义；集群管理 actor 作为单一桥梁，边界明确。
- **兼容与渐进**：GET/PUT、nodes 表、Web 页不变；Phase 1→2→3 可分批落地，风险可控。
- **发现路径完整**：已初始化 / 未初始化且配置 init 两条路径都覆盖，文档里写清楚了分支逻辑。

### 潜在问题与风险

1. **发现时机与竞态**
   「适当时机」若只在 server 启动或首次访问时做一次发现，之后 Pulsing 才被用户 `pul.init()`，可能漏掉。建议：发现失败或未发现时，在后续**定时重试**或「首次访问集群视图时重试」，并文档化重试策略（间隔、上限）。

2. **集群管理 actor 的归属与生命周期**
   actor 由 probing 在「别人的」ActorSystem 上注册，若 probing 先退出而 Pulsing 常驻，会留下 named actor；若 Pulsing 先 shutdown，probing 侧要有**检测断开 + 回退到仅上报**的逻辑，否则会持续请求已失效的 actor。文档可明确：actor 随哪边生命周期、断开后是否自动降级。

3. **probing 自己 init Pulsing 的适用边界**
   由 probing 调 `pul.init(seeds=...)` 时，当前进程会加入 Pulsing 集群。若同一台机上多进程都注入 probing 且都配置了 init，可能变成多节点加入同一集群，是否预期需要写清；standalone 与 cluster 模式在「由 probing 初始化」时的行为建议在文档里区分（例如仅允许 standalone 或仅允许指定了 seeds 的 cluster）。

4. **身份对应 (host:addr vs node_id)**
   Pulsing 的 `node_id` 与 probing 的 `host:addr` 如何稳定对应，文档只说了「约定」。若同一物理机多进程、或容器重启导致 addr 复用，合并时可能错位或重复。建议在实施时约定：例如优先用 `addr` 做关联，或要求上报里带 `pulsing_node_id` 以便精确匹配，并在设计里写一句「合并键策略」。

5. **Rust 与 Python 的边界**
   cluster 模块若在 Rust（server）里，发现「全局 ActorSystem」必然要经 Python 或 HTTP；若在 Python 扩展里，则可直接 `pul.get_system()`。文档里「发现」和「注册 actor」到底在 Rust 侧还是 Python 侧实现，会直接影响 Phase 1 的落地方式，建议在实施步骤里明确**谁负责发现、谁负责注册 actor、谁负责拉取并写 CLUSTER**（例如：Python 发现+注册 actor，Rust 通过 FFI/HTTP 向该 actor 要成员并写 CLUSTER）。

### 建议补充

- **配置契约**：列出「发现 / 初始化」相关配置项与默认值（如 `pulsing_auto_discover=true`、未配置 seeds 时不自动 init），避免实现时各说各话。
- **可观测**：发现成功/失败、是否使用集群管理 actor、降级到仅上报，建议打日志或指标，便于运维判断当前是否在用 Pulsing 视图。
- **测试策略**：无 Pulsing、已有 Pulsing、probing 自己 init 三种场景各有一个明确测试用例，防止回归。

整体上，这个实现方向合理、文档已把主流程和松耦合讲清楚；把上述「发现重试、生命周期与降级、身份对应、Rust/Python 边界」在文档或后续实现里补上，落地会更稳。

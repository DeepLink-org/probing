# torchrun 分层集群心跳

多进程 `torchrun` 作业在 probing 注入后**默认自动**注册集群节点，供 `probing cluster nodes`、Web 集群页与 `global.*` 联邦查询使用。实现位于 L3 `probing/server`（Rust），**不阻塞** `init_process_group`，**不写入** torch rendezvous key。

## 启动条件

| 条件 | 说明 |
|------|------|
| `PROBING=1/2` | 进程已注入 probing |
| `WORLD_SIZE > 1` | 单进程不启动 |
| `PROBING_TORCHRUN_CLUSTER≠0` | 默认 **开** |
| `PROBING_CLUSTER_REPORT≠0` | 默认 **开** |
| 非 elastic supervisor | torchrun 主进程不绑 HTTP |

满足条件时，Rust ctor（`import probing`）调用 `maybe_start_torchrun_cluster()`：绑 HTTP、在 TCPStore 发布 master/local0 地址、启动 Tokio 心跳 worker。

## 分层拓扑

```text
leaf (local_rank>0)  ──PUT──►  local0 (本机 local_rank=0)
local0 (非 global0) ──PUT──►  master (global rank 0)
global rank 0       ──PUT──►  本机 master 视图
```

- **发现**：`probing/torchrun/<run_id>/master` 与 `.../node/<group_rank>/local0` 写在 job TCPStore（与 torch rendezvous 同 endpoint，独立 key 前缀）。
- **合并**：`PUT /apis/nodes` 合并心跳；`GET /apis/nodes` 返回按 rank 排序的快照。
- **收敛**：未凑齐全员时保持 **base 间隔**（默认 10s）；全员 alive 后指数退避（默认 ×2，有上限）。

## 环境变量

### 开关

| 变量 | 默认 | 说明 |
|------|------|------|
| `PROBING_TORCHRUN_CLUSTER` | `1` | `0` 关闭 torchrun 集群 |
| `PROBING_CLUSTER_REPORT` | `1` | `0` 只绑 HTTP，不跑周期心跳 |
| `PROBING_CLUSTER_REPORT_BACKOFF` | `1` | `0` 关闭稳定后退避 |

### 心跳与 TTL

| 变量 | 默认 | 说明 |
|------|------|------|
| `PROBING_CLUSTER_REPORT_INTERVAL_SEC` | `10` | 基础心跳间隔（秒） |
| `PROBING_CLUSTER_REPORT_MAX_INTERVAL_SEC` | `120` | 退避上限（会被 stale 安全钳制） |
| `PROBING_CLUSTER_REPORT_BACKOFF_FACTOR` | `2` | 每次稳定 tick 的乘数 |
| `PROBING_CLUSTER_STALE_SEC` | `25` | 超时未心跳则标 `dead` |
| `PROBING_CLUSTER_DISCOVER_TIMEOUT_SEC` | `2` | 发现 master/local0 单次超时 |
| `PROBING_CLUSTER_REPORT_TIMEOUT_SEC` | `5` | PUT 超时 |

**stale 与退避**：实际 max 间隔 = `min(配置上限, STALE_SEC - STALE_SEC/4 - 1)`。默认 stale=25 时 max ≈ **18s**。若希望稳定后 60s 心跳，需同时提高 `PROBING_CLUSTER_STALE_SEC`（建议 ≥90）。

### 网络

| 变量 | 说明 |
|------|------|
| `PROBING_PORT` | 仅 **global rank 0** 绑定；其他 rank 用 `0.0.0.0:0` |
| `MASTER_ADDR` / `MASTER_PORT` | TCPStore endpoint（torchrun 已设置） |
| `RDZV_ID` | 多 `torchrun` 并行时必须共享（见 demo 脚本） |

## 预设（`PROBING_CLUSTER_PRESET`）

`examples/run_cluster_multinode.sh` 支持预设，便于 demo / 调试 / 长跑：

| 预设 | 用途 | 主要效果 |
|------|------|----------|
| `demo`（默认） | 本地多机 demo | 默认 env，10s 收敛 + 短 stale |
| `fast` | 快速看清 cluster 收敛 | interval=5s，stale=30s |
| `steady` | 长跑省 CPU | stale=90s，max interval≈67s |

```bash
PROBING_CLUSTER_PRESET=fast ./examples/run_cluster_multinode.sh 2 2
PROBING_CLUSTER_PRESET=steady ./examples/run_cluster_multinode.sh 4 8
```

也可手动覆盖任意变量；预设只设置未显式 export 的项。

## 查询与 role

```bash
# 连 global rank 0（PROBING_PORT，默认 18080）
probing -t rank0-host:18080 cluster nodes
```

训练脚本中 `probing.set_role(dp=…)` 后调用 `refresh_node_role()`（Python facade）立即补一条心跳。

## 演示

```bash
./examples/run_cluster_multinode.sh        # 2 机 × 2 卡
./examples/run_cluster_multinode.sh 3 4 60 # 3 机 × 4 卡，sleep 60s
```

## 与 Python hook 的关系

集群心跳**仅由 Rust ctor 启动**；不再 patch `torch.distributed.init_process_group`。Python 模块 `probing.torchrun_cluster` 保留 `setup_torchrun_cluster()` / `master_info()` 供显式调用与测试。

## 相关文档

- [分布式架构](distributed.zh.md) — 联邦查询与 cluster API
- [环境变量](../reference/env-vars.md) — 完整 env 列表

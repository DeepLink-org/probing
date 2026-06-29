# 环境变量

Probing 读取的全部 `PROBING_*` 环境变量参考（按子系统分组）。英文版见 [Environment Variables](env-vars.md)。

## 激活

| 变量 | 取值 | 默认 | 说明 |
|----------|--------|---------|-------------|
| `PROBING` | `0`, `1`/`followed`, `2`/`nested`, `regex:PATTERN`, `SCRIPT.py` | 未设置（禁用） | 是否启用 probing。`1` 仅当前进程；`2` 当前及子进程；`regex:` 脚本名匹配时启用。 |
| `PROBING_ORIGINAL` | （自动设置） | — | 备份原始 `PROBING` 值；由 site_hook 设置，勿手动设置。 |

## 集群 {#集群}

`WORLD_SIZE > 1` 时的分层 side-channel 注册。详见 [torchrun 集群心跳](../design/torchrun-cluster.zh.md) 与 [分层 fan-out](../design/hierarchical-fanout.zh.md)。

| 变量 | 默认 | 说明 |
|----------|---------|-------------|
| `PROBING_CLUSTER_REPORT` | `1` | 周期性心跳 worker；`0` = 仅 HTTP，无周期 PUT。 |
| `PROBING_CLUSTER_REPORT_INTERVAL_SEC` | `10` | 基础心跳间隔（秒）。 |
| `PROBING_CLUSTER_REPORT_MAX_INTERVAL_SEC` | `120` | 退避上限（低于 stale TTL）。 |
| `PROBING_CLUSTER_REPORT_BACKOFF_FACTOR` | `2` | 稳定 tick 的倍增因子。 |
| `PROBING_CLUSTER_REPORT_BACKOFF` | `1` | 设为 `0` 禁用稳定时的指数退避。 |
| `PROBING_CLUSTER_STALE_SEC` | `25` | 无心跳超过此秒数标记为 `dead`；应大于最大间隔。 |
| `PROBING_CLUSTER_DISCOVER_TIMEOUT_SEC` | `2` | 每次 master/local0 发现超时。 |
| `PROBING_CLUSTER_REPORT_TIMEOUT_SEC` | `5` | 集群 report HTTP PUT 超时。 |
| `PROBING_CLUSTER_PRESET` | — | `examples/run_cluster_multinode.sh` 使用：`demo`、`fast`、`steady`。 |
| `PROBING_CLUSTER_FANOUT_HIERARCHICAL` | `1` | 分层集群查询 fan-out；`0` = 扁平 fan-out 到所有 peer。 |
| `PROBING_REMOTE_QUERY_TIMEOUT_SECS` | `2` | 远程联邦 / 集群查询的单 peer 超时（秒）。 |

## 其余变量

激活、存储、Server、认证、Tracing、采样、NCCL、RDMA、PyTorch、调试等章节与 [英文 env-vars](env-vars.md) 同步；尚未单独翻译。

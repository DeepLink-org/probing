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
| `PROBING_CLUSTER_STALE_SEC` | `25` | 无心跳超过一个 TTL 标记为 `dead`，再经过一个 TTL 后删除；应大于最大间隔。 |
| `PROBING_CLUSTER_DISCOVER_TIMEOUT_SEC` | `2` | 每次 master/local0 发现超时。 |
| `PROBING_CLUSTER_REPORT_TIMEOUT_SEC` | `5` | 集群 report HTTP PUT 超时。 |
| `PROBING_CLUSTER_PRESET` | — | `examples/cluster/run_multinode.sh` 使用：`demo`、`fast`、`steady`。 |
| `PROBING_CLUSTER_FANOUT_HIERARCHICAL` | `1` | 分层集群查询 fan-out；`0` = 扁平 fan-out 到所有 peer。 |
| `PROBING_REMOTE_QUERY_TIMEOUT_SECS` | `30` | 远程联邦 / 集群查询的单 peer 超时（秒）。 |
| `PROBING_FANOUT_CONCURRENCY` | `128` | 单次 cluster fan-out 的最大并发远程 HTTP 请求数。 |
| `PROBING_STACK_FANOUT_CONCURRENCY` | `32` | Distributed stacks 同时采集的最大 peer 数。 |
| `PROBING_STACK_FANOUT_DEADLINE_SEC` | `15` | Distributed stacks 的整体 fan-out 截止时间；超时后返回已完成 peer 的部分火焰图并列出失败 peer。 |
| `PROBING_ADVERTISE_ADDR` | `MASTER_ADDR`，否则 hostname | wildcard bind 时向 peer 发布的地址；支持 `host`、`host:port`、IPv6 或 `{port}` 占位符。多网卡环境或 `MASTER_ADDR` 不是当前节点的 peer 可达地址时必须显式设置。 |
| `PROBING_NODE_HOST` | 操作系统 hostname | cluster heartbeat 中上报的显式 host 标签。用于容器身份和本地逻辑节点 fixture；不会改变向 peer 发布的网络地址。 |
| `PROBING_NCCL_CHUNK_BYTES` | `65536` | NCCL profiler mmap 环缓冲 chunk 大小（字节）。 |
| `PROBING_NCCL_NUM_CHUNKS` | `64` | NCCL profiler mmap 环缓冲 chunk 数量（默认每表约 4 MiB）。 |
| `PROBING_NCCL_MAX_COLL_SLOTS` | `512` | 每 rank 最大 in-flight collective/P2P slot 数。 |
| `PROBING_NCCL_MAX_PROXY_OP_SLOTS` | `8192` | 每 rank 最大 proxy-op slot 数。 |
| `PROBING_NCCL_MAX_PROXY_STEP_SLOTS` | `32768` | 每 rank 最大 proxy-step slot 数。 |
| `PROBING_NCCL_MAX_KERNEL_CH_SLOTS` | `8192` | 每 rank 最大 kernel-channel slot 数。 |
| `PROBING_NCCL_MAX_NET_SLOTS` | `4096` | 每 rank 最大 net-plugin slot 数。 |
| `PROBING_NCCL_POOL_SHARDS` | `8` | 按 comm hash 分片 slot pool（1–64）；总 slot 上限均分到各 shard。 |
| `PROBING_NCCL_MIN_MSG_BYTES` | `0` | 低于此消息大小（字节）的事件不记录；`0` = 全记录。 |

## 其余变量

激活、存储、Server、认证、Tracing、采样、NCCL、RDMA、PyTorch、调试等章节与 [英文 env-vars](env-vars.md) 同步；尚未单独翻译。

`PROBING_TCPSTORE_INSPECT` 默认为 `0`。设为 `1` 后，
`pytorch/runtime-debug?include_values=true` 才能预览默认打码的 TCPStore value；
接口仍为只读，仅应在可信环境启用。

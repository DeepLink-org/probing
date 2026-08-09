# 环境变量

Probing 读取的全部 `PROBING_*` 环境变量参考（按子系统分组）。英文版见 [Environment Variables](env-vars.md)。

## 激活

| 变量 | 取值 | 默认 | 说明 |
|----------|--------|---------|-------------|
| `PROBING` | `0`, `1`/`followed`, `2`/`nested`, `regex:PATTERN`, `SCRIPT.py` | 未设置（禁用） | 是否启用 probing。`1` 仅当前进程；`2` 当前及子进程；`regex:` 脚本名匹配时启用。 |
| `PROBING_ORIGINAL` | （自动设置） | — | 备份原始 `PROBING` 值；由 site_hook 设置，勿手动设置。 |

## 数据存储 {#data-storage}

| 变量 | 默认 | 说明 |
|------|------|------|
| `PROBING_DATA_DIR` | 平台相关 | MEMT mmap 文件根目录；每个进程使用 PID 子目录。 |
| `PROBING_TABLE_DEFAULT_MB` | `20` | Python `@table` 与未指定容量的 `ExternalTable` 默认 mmap 环形容量。 |
| `PROBING_COLD` | 未设置 | 设为 `on` 启用 MEMT 到 MEMC 的后台整理。 |
| `PROBING_COLD_TARGET_MB` | — | 冷段目标滚动大小。 |
| `PROBING_COLD_MAX_TOTAL_MB` | — | 冷层总字节预算。 |
| `PROBING_COLD_TTL_SECS` | — | 冷段保留时间。 |
| `PROBING_COLD_POLL_MS` | — | Compactor 两轮扫描之间的间隔。 |
| `PROBING_COLD_MAX_AGE_SECS` | — | 打开段达到该年龄后强制封存。 |
| `PROBING_COLD_DIR` | `PROBING_DATA_DIR` 下 | 冷段目录。 |

## Tracing 与 Span {#tracing-spans}

| 变量 | 默认 | 说明 |
|------|------|------|
| `PROBING_SPAN_BACKENDS` | `memtable` | 逗号分隔的 backend：`memtable`、`logger`、`otel`、`none`。`none` 仅保留栈，不持久化。详见 [Tracing 与训练阶段](../design/profiling.zh.md#span-api)。 |
| `PROBING_SPAN_LOG_LEVEL` | `INFO` | `logger` backend 的日志级别。 |
| `PROBING_SPAN_LOCATION` | 未设置 | 为每个 span 通过 `inspect.stack()` 采集位置，开销较高。 |
| `PROBING_TRACE_STDOUT` | 未设置 | `1`/`true` 时让 `probing.inspect.trace` 输出到 stdout，而不是 Python logger。 |

## 集群 {#集群}

`WORLD_SIZE > 1` 时的分层 side-channel 注册。详见 [分布式成员](../design/distributed.zh.md#cluster-membership) 与 [联邦查询 — 分层 fan-out](../design/federation.zh.md#hierarchical-fan-out)。

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
| `PROBING_FANOUT_WORKER_THREADS` | `4` | 分布式 SQL、Extension 采集、节点发现和心跳共用的独立异步 fan-out runtime 线程数。 |
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

## Megatron 自动集成 {#megatron-autostart}

检测到 Megatron 环境变量或模块后，集成以 best-effort 方式自动启用；除 `PROBING=2` 外，
无需修改训练脚本。

| 变量 | 默认 | 说明 |
|------|------|------|
| `PROBING_MEGATRON` | `auto` | `auto` 表示检测到 Megatron 环境或模块时启用；也可用 `on`/`off` 强制开关。 |
| `PROBING_MEGATRON_STEP_SYNC` | `auto` | 包装 `train_step`，将 `probing.step` 与 Megatron iteration 对齐。 |
| `probing.megatron.enable` | — | 通过 `probing.config.set` 覆盖自动集成开关。 |
| `probing.megatron.step_sync` | — | 通过 `probing.config.set` 覆盖 iteration 同步开关。 |

当 `megatron.core.parallel_state` 和 `megatron.training.training` 加载时，import hook
把并行 rank 写入 `probing.set_role`，并让 `train_step` 产生可供 SQL 关联的统一 step 坐标。

## 其余变量

激活、存储、Server、认证、Tracing、采样、NCCL、RDMA、PyTorch、调试等章节与 [英文 env-vars](env-vars.md) 同步；尚未单独翻译。

`PROBING_TCPSTORE_INSPECT` 默认为 `0`。设为 `1` 后，
`pytorch/runtime-debug?include_values=true` 才能预览默认打码的 TCPStore value；
接口仍为只读，仅应在可信环境启用。

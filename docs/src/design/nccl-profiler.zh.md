# NCCL profiler 插件

面向分布式训练的 **NCCL 等待分解**：区分 **culprit**（本 rank GPU 产出慢）与 **victim**（等待 peer / 网络）。

属于 [扩展机制](extensibility.zh.md) 中的 **路径 3**——由 NCCL 加载的 Rust `cdylib`，不是 Python 表插件。

## 何时使用

| 现象 | 工具 |
|------|------|
| step 变慢，不确定是通信还是计算 | `python.comm_collective` + skill `comm_bottleneck` |
| 哪个 rank 是 straggler？ | skill `slow_rank` |
| 已定位慢 rank，要区分 GPU 慢还是等网络 | `nccl.proxy_ops` + skill `nccl_culprit_victim` |
| 怀疑 RoCE / IB 拥塞 | `nccl.net_qp` + `rdma.mlx_hca` |

粗粒度 collective（`python.comm_collective`）只需 `PROBING=1`。NCCL profiler 插件需要 **NCCL ≥ 2.26**（建议 PyTorch **2.8+**）；插件同时导出 **`ncclProfiler_v4`**（NCCL ≥ 2.27，优先）与 **`ncclProfiler_v3`**（NCCL 2.26），NCCL 自动协商最高版本。

## 三条 collective 采集路径 — 保持分叉，勿混淆

probing 有三条相互独立的集合通信采集路径，**计时语义不同**，不可混用：

| 来源 | 表 | 测的是什么 | 定位 |
|------|----|-----------|------|
| **NCCL profiler 插件**（本文档） | `nccl.coll_perf`、`nccl.proxy_ops`、`nccl.inflight_ops`、`nccl.net_qp` | NCCL 原生事件：重建的执行时间、等待分解、带宽 | **精准数据源** |
| Torch API 层插桩（遗留，`probing/profiling/collective/`） | `python.comm_collective` | `torch.distributed` API 调用的 Python 墙钟（launch 层） | 粗粒度回退；独有 `global_step` 上下文 |
| PyTorch Flight Recorder 桥 | `python.torch_nccl_flight_record`、`python.torch_nccl_pg_status` | torch 内部 watchdog 环形缓冲 | watchdog timeout / desync 取证 |

约定：

- 插件启用时（设置了 `NCCL_PROFILER_PLUGIN`），Torch API 层插桩**默认关闭**——
  同一批 collective 被两套语义各记一遍只会制造混乱。如需同开（例如要按
  `global_step` 对齐），`SET probing.torch.collective.enable=1`。
- 查执行时间、带宽、等待归因，一律用 `nccl.*`。
  `python.comm_collective.duration_ms` **不是** NCCL 执行时间——`async_op`
  调用在 `work.wait()` 处收口，同步调用在 API 返回处收口。
- 跨层关联：`nccl.*` 行不带训练步；需要时用 epoch 纳秒时间窗对
  `python.comm_collective.global_step` 对齐。

## 快速开始（Linux 训练）

```bash
pip install probing   # Linux wheel 自带 libprobing_nccl_profiler.so

export NCCL_PROFILER_PLUGIN=$(python -m probing.nccl --plugin-path)
export NCCL_PROFILE_EVENT_MASK=$(python -m probing.nccl --event-mask)   # 默认 94
export PROBING=2

torchrun --nproc_per_node=8 train.py

probing -t <pid> skill run nccl_culprit_victim
probing -t <pid> query "
  SELECT rank, sum(send_gpu_wait_ns) AS gpu_wait, sum(recv_wait_ns) AS recv_wait
  FROM nccl.proxy_ops
  GROUP BY rank
  ORDER BY recv_wait DESC"
```

### 可选：NetPlugin（IB QP 时延）

```bash
export NCCL_PROFILE_EVENT_MASK=222   # 94 + NetPlugin 位 128
probing -t <pid> query "SELECT * FROM nccl.net_qp LIMIT 20"
```

## macOS / 无 NCCL 开发机

```bash
PROBING=1 PROBING_NCCL_MOCK=1 python -m probing.nccl --seed-mock
probing -t <pid> skill run nccl_culprit_victim
```

macOS 默认 `PROBING_NCCL_MOCK=auto`：在 `PROBING=1` 且无插件 `.so` 时自动写入 mock 表。

Mock 场景：**rank 2** = culprit（`send_gpu_wait_ns` 高），**rank 5** = victim（`recv_wait_ns` 高）。

## 数据表

### `nccl.proxy_ops`

每个 NCCL proxy op 一行，ProxyStep 等待在 op 结束时聚合。

| 列 | 含义 |
|----|------|
| `ts` | 时间戳（纳秒） |
| `rank` | `torch.distributed` rank |
| `tp_rank`, `pp_rank`, `dp_rank` | 并行角色（`TP_RANK` / `PP_RANK` / `DP_RANK` 等 env）；未设置则为 `-1` |
| `comm_hash` | NCCL communicator hash |
| `coll_func` | collective 名称 |
| `seq` | collective 序号 |
| `channel_id` | NCCL channel |
| `peer` | 对端 rank |
| `is_send` | `1` 发送 proxy，`0` 接收 |
| `n_steps` | 聚合的 ProxyStep 数 |
| `trans_bytes` | 传输字节数（v4 按 step 级 `transSize` 累计） |
| `send_gpu_wait_ns` | **culprit 信号** — 本 GPU 未就绪 |
| `send_peer_wait_ns` | 等待接收端 clear-to-send credits（**仅 v4 ABI**，v3 为 0）— 对端拥塞信号 |
| `send_wait_ns` | 发送侧网络等待 |
| `recv_wait_ns` | **victim 信号** — 等待对端数据 |
| `recv_flush_wait_ns` | 接收 flush 等待 |

多机：`global.nccl.proxy_ops`，带 `_host`、`_addr`、`_rank`。

> 所有 `nccl.*` 表的 `ts` 均为 **UNIX epoch 纳秒**，`global.nccl.*` 跨 rank 查询时时间戳可直接比较。

### `nccl.coll_perf`

每个 collective / P2P 操作一行。

**计时模型。** NCCL 官方文档明确：collective 的 `stopEvent` 只表示 **host 侧
enqueue 结束**——内核与 proxy 线程在其后继续工作。插件按官方 ext-profiler
推荐做法，对子事件（`ProxyOp`、`KernelCh`）做引用计数，并用子事件窗口重建
真实执行时间。`timing_source` 列标注实际使用的信号：

| `timing_source` | 窗口 | 质量 |
|-----------------|------|------|
| `kernel_gpu` | GPU **globaltimer** 窗口：`kernelCh.pTimer`（起点）+ `KernelChStop` 状态（终点） | 最佳 — 设备时钟，**仅 v4 ABI** |
| `kernel_ch` | proxy 线程观测到的内核活动窗口（`ncclProfileKernelCh`） | NCCL 自身内核活动信号，host 时钟 |
| `proxy` | proxy op start→stop 包络 | 跨机操作较准 |
| `enqueue` | coll start→stop（仅 launch） | 回退 — 无 proxy/kernel 事件的机内操作 |

| 列 | 含义 |
|----|------|
| `ts` | 完成时间戳（epoch 纳秒） |
| `rank`, `tp_rank`, `pp_rank`, `dp_rank` | 同 `nccl.proxy_ops` |
| `comm_hash`, `coll_func`, `seq` | 操作标识（P2P 的 `seq` 为 0） |
| `n_ranks` | 通信组大小（v4 per-comm `init` 提供；v3 为 `-1`） |
| `is_p2p` | `1` = Send/Recv，`0` = collective |
| `peer` | P2P 对端 rank（collective 为 `-1`） |
| `count`, `msg_size_bytes`, `dtype` | 消息负载：元素数 × dtype 字节数 |
| `algo`, `proto`, `n_channels` | NCCL 算法（Ring/Tree…）、协议（LL/LL128/Simple）、channel 数（v4 起 P2P 也有值） |
| `exec_time_ns` | 重建的真实执行耗时（见 `timing_source`） |
| `enqueue_time_ns` | host 侧 enqueue 耗时（NCCL coll start→stop） |
| `timing_source` | `kernel_gpu` / `kernel_ch` / `proxy` / `enqueue` |
| `algobw_gbps` | 算法带宽 `msg_size / exec_time`（GB/s）。**busbw** 在 SQL 中用 `n_ranks` 乘集合通信系数（如 AllReduce `2(n_ranks-1)/n_ranks`） |

```sql
-- 按带宽找最慢的 collective 分桶
SELECT coll_func, msg_size_bytes, AVG(algobw_gbps) AS gbps, COUNT(*) AS n
FROM nccl.coll_perf
GROUP BY coll_func, msg_size_bytes
ORDER BY gbps ASC LIMIT 10
```

### `nccl.inflight_ops`

watchdog 周期快照：**已 start 未 stop** 的操作——挂死的 op 永远不会触发
`stop_event`，因此不会出现在 `nccl.proxy_ops` 里；这张表补上了 hang 场景的盲区。
列：`ts`, `rank`, `comm_hash`, `coll_func`, `seq`, `kind`（`coll`/`p2p`/`proxy_op`）,
`channel_id`, `peer`, `is_send`, `start_ns`, `age_ns`。

```sql
-- 哪个 rank 卡在哪个操作上？
SELECT rank, coll_func, seq, kind, MAX(age_ns)/1e9 AS stuck_secs
FROM nccl.inflight_ops
GROUP BY rank, coll_func, seq, kind
ORDER BY stuck_secs DESC
```

### `nccl.net_qp`

IB QP 完成时延（需 NetPlugin mask）。列：`ts`, `rank`, `device`, `qp_num`, `wr_id`, `opcode`, `length`, `duration_ns`。

## Culprit 与 Victim

- **Culprit**：某 rank `send_gpu_wait_ns` 突出 → 本地 GPU/计算慢，拖慢 collective 产出。
- **Victim**：某 rank `recv_wait_ns` 突出 → 在等他人或网络。

同一 rank 可能在不同 collective 上同时出现两种模式。结合 `tp_rank`/`pp_rank`/`dp_rank` 与 Megatron 拓扑对齐分析。

## 诊断 skill：`nccl_culprit_victim`

目录：`skills/nccl_culprit_victim/`（wheel 内：`python/probing/_skills/`）。

```bash
probing skill list
probing -t <pid> skill run nccl_culprit_victim
probing -t <pid> skill run nccl_culprit_victim --set seq_window=50 --global
```

步骤包括：各 rank wait 汇总、culprit/victim 排行、tp/pp/dp 角色视图、可选 `global` fan-out 与 `nccl.net_qp` 提示。

关联 skill：`slow_rank`、`comm_bottleneck`（粗粒度；有 `nccl.proxy_ops` 时会附带 NCCL 步骤）。

## 环境变量

| 变量 | 作用 |
|------|------|
| `NCCL_PROFILER_PLUGIN` | `libprobing_nccl_profiler.so` 路径 |
| `NCCL_PROFILE_EVENT_MASK` | 事件 mask；默认 `94` = Coll \| P2P \| ProxyOp \| ProxyStep \| KernelCh |
| `PROBING_DATA_DIR` | memtable 目录 |
| `PROBING_NCCL_MIN_MSG_BYTES` | 小于该字节数的操作不记录；默认 `0`（全记）。对应 NCCL Inspector 的 `DUMP_MIN_SIZE_BYTES` |
| `PROBING_NCCL_INFLIGHT_THRESHOLD_SECS` | watchdog：在途超过该秒数的操作快照进 `nccl.inflight_ops`；默认 `10`，`0` 关闭 |
| `PROBING_NCCL_POOL_SHARDS` | 按 comm hash 分片 slot pool（默认 `8`，范围 1–64）；降低多 comm 场景下回调锁竞争 |
| `PROBING_NCCL_MOCK` | 开发 mock：`auto` / `1` / `0` |
| `TP_RANK`, `PP_RANK`, `DP_RANK` | 写入 proxy_ops 角色列 |

```bash
python -m probing.nccl --plugin-path
python -m probing.nccl --event-mask
python -m probing.nccl --seed-mock --ranks 8 --ops 5
```

## 源码构建

```bash
make nccl-profiler-lib
cargo test -p probing-nccl-profiler
```

实现：`probing/extensions/nccl-profiler/`。架构细节见 crate [README](https://github.com/DeepLink-org/probing/blob/main/probing/extensions/nccl-profiler/README.md)。

## 真机验收清单（P0）

1. 确认 NCCL ≥ 2.26
2. `torchrun` 前设置 `NCCL_PROFILER_PLUGIN`
3. 若干 collective 后：`SELECT count(*) FROM nccl.proxy_ops` > 0
4. `probing skill run nccl_culprit_victim` 有 rank 分解结果

## 相关文档

- [分布式训练](distributed.zh.md)
- [扩展机制](extensibility.zh.md)
- [AGENTS.md](https://github.com/DeepLink-org/probing/blob/main/AGENTS.md)

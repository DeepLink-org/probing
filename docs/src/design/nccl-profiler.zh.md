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

粗粒度 collective（`python.comm_collective`）只需 `PROBING=1`。NCCL profiler 插件需要 **NCCL ≥ 2.26**（建议 PyTorch **2.8+**）。

## 快速开始（Linux 训练）

```bash
pip install probing   # Linux wheel 自带 libprobing_nccl_profiler.so

export NCCL_PROFILER_PLUGIN=$(python -m probing.nccl --plugin-path)
export NCCL_PROFILE_EVENT_MASK=$(python -m probing.nccl --event-mask)   # 默认 26
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
export NCCL_PROFILE_EVENT_MASK=154   # 26 + NetPlugin 位 128
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
| `trans_bytes` | 传输字节数 |
| `send_gpu_wait_ns` | **culprit 信号** — 本 GPU 未就绪 |
| `send_wait_ns` | 发送侧网络等待 |
| `recv_wait_ns` | **victim 信号** — 等待对端数据 |
| `recv_flush_wait_ns` | 接收 flush 等待 |

多机：`global.nccl.proxy_ops`，带 `_host`、`_addr`、`_rank`。

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
| `NCCL_PROFILE_EVENT_MASK` | 事件 mask；默认 `26` = Coll \| ProxyOp \| ProxyStep |
| `PROBING_DATA_DIR` | memtable 目录 |
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

# NCCL profiler 性能开销实验

> 实验日期：2026-08-03  
> 实验节点：`gpu-l-lg-cmc-h-h200-0362.host.h.pjlab.org.cn`  
> 远端工作目录：`/mnt/shared-storage-user/ailab-sys/zhangwensheng/probing`

## 1. 目的

本实验评估启用 `probing-nccl-profiler` 对以下两类工作负载的性能影响：

1. 固定消息大小的 NCCL AllReduce 微基准；
2. 开启 `PROBING_TORCH_PROFILING=rate=0.01,backward=on` 的真实
   Megatron-Core 训练。

这里必须区分两个独立开销来源：

- `PROBING_TORCH_PROFILING=rate=0.01` 控制 Torch 模块 hook 的 step
  采样率；`0.01` 表示约 1% 的 step 被采样。
- `NCCL_PROFILER_PLUGIN` 不使用该采样率。插件会持续接收满足 event mask
  的 NCCL 回调并写入 `nccl.*` memtable。

因此，`python.torch_step_timing` 中 sampled/normal/shadow step 的差异只能衡量
Torch profiling 开销，不能直接归因给 NCCL profiler。NCCL profiler 的开销必须
通过插件关闭/开启的 A/B 实验测量。

## 2. 实验环境

| 项目 | 配置 |
|---|---|
| GPU | 8 × NVIDIA H200，单节点 |
| PyTorch | 2.8.0+cu126 |
| CUDA | 12.6 |
| NCCL | 2.27.3 |
| NCCL profiler ABI | v4 |
| NCCL event mask | 94（Coll、P2P、ProxyOp、ProxyStep、KernelCh） |
| 插件 | `libprobing_nccl_profiler.so` |
| Megatron 拓扑 | TP=2、PP=1、DP=4 |
| Torch profiling | `rate=0.01,backward=on` |

本实验是单节点 NVLink 实验。它覆盖 collective 和 KernelCh 等事件开销，但不覆盖
真实跨节点 IB/RoCE 路径上的 `proxy_ops`、`net_qp` 和 NetPlugin 事件开销。

## 3. 插件加载方式

NCCL 要求 `NCCL_PROFILER_PLUGIN` 使用 `.so` 裸文件名，插件目录通过
`LD_LIBRARY_PATH` 提供。不能把绝对路径直接赋给 `NCCL_PROFILER_PLUGIN`。

```bash
cd /mnt/shared-storage-user/ailab-sys/zhangwensheng/probing

export LD_LIBRARY_PATH="$PWD/python/probing/libs:/usr/local/nvidia/lib64:${LD_LIBRARY_PATH:-}"
export NCCL_PROFILER_PLUGIN=libprobing_nccl_profiler.so
export NCCL_PROFILE_EVENT_MASK=94
```

## 4. NCCL AllReduce 微基准

### 4.1 方法

基准程序：

```text
examples/overhead/nccl_profiler_overhead.py
```

每个消息大小执行三组 baseline/profiled 配对：

- 8 个 rank；
- 100 次 warmup；
- 500 次计时迭代；
- 消息大小分别为 1 MiB、16 MiB、256 MiB；
- baseline 与 profiled 均设置 `PROBING=2`；
- 两组之间唯一关键差异是是否设置 `NCCL_PROFILER_PLUGIN`；
- profiled 使用 event mask 94，并关闭 inflight watchdog，避免额外噪声。

代表性 baseline 命令：

```bash
env -u NCCL_PROFILER_PLUGIN -u NCCL_PROFILE_EVENT_MASK \
  PROBING=2 \
  python3.12 -m torch.distributed.run \
    --standalone --nproc_per_node=8 \
    examples/overhead/nccl_profiler_overhead.py \
    --warmup-iters 100 \
    --bench-iters 500 \
    --msg-bytes 16777216 \
    --output /tmp/nccl_overhead_exp/base_16777216_1.json
```

代表性 profiled 命令：

```bash
PROBING=2 \
PROBING_DATA_DIR=/tmp/nccl_overhead_exp/data_16777216_1 \
PROBING_NCCL_INFLIGHT_THRESHOLD_SECS=0 \
NCCL_PROFILER_PLUGIN=libprobing_nccl_profiler.so \
NCCL_PROFILE_EVENT_MASK=94 \
python3.12 -m torch.distributed.run \
  --standalone --nproc_per_node=8 \
  examples/overhead/nccl_profiler_overhead.py \
  --warmup-iters 100 \
  --bench-iters 500 \
  --msg-bytes 16777216 \
  --output /tmp/nccl_overhead_exp/prof_16777216_1.json
```

延迟开销定义为：

```text
latency_overhead = (profiled_latency / baseline_latency - 1) × 100%
```

### 4.2 原始结果

单位为平均 AllReduce 延迟（µs）。

| 消息大小 | 重复 | Baseline | Profiled | 配对变化 |
|---|---:|---:|---:|---:|
| 1 MiB | 1 | 39.426 | 46.438 | +17.79% |
| 1 MiB | 2 | 53.056 | 47.062 | -11.30% |
| 1 MiB | 3 | 54.915 | 71.425 | +30.06% |
| 16 MiB | 1 | 138.981 | 149.671 | +7.69% |
| 16 MiB | 2 | 162.049 | 143.164 | -11.65% |
| 16 MiB | 3 | 142.716 | 143.744 | +0.72% |
| 256 MiB | 1 | 1099.922 | 1099.257 | -0.06% |
| 256 MiB | 2 | 1087.302 | 1086.167 | -0.10% |
| 256 MiB | 3 | 1091.301 | 1613.342 | +47.84% |

### 4.3 稳健汇总

| 消息大小 | 配对开销中位数 | 吞吐变化中位数 | 解释 |
|---|---:|---:|---|
| 1 MiB | +17.79% | -15.10% | 绝对延迟很短，固定回调成本和系统抖动均明显；当前数据不足以定量 |
| 16 MiB | +0.72% | -0.72% | 约 1% 以内 |
| 256 MiB | -0.06% | +0.06% | 前两次几乎一致；第三次 profiled 是明显系统离群点 |

256 MiB 第三次 profiled 为 1613.342 µs，而其他五次都在约
1086–1100 µs。原始结果没有删除该数据；稳健结论使用配对中位数并明确标记
该离群点。

## 5. Megatron-Core 端到端 A/B

### 5.1 方法

最终使用的训练配置：

- 8 卡；
- TP=2、PP=1、DP=4；
- 每次固定 200 iterations；
- baseline 与 profiled 均启用
  `PROBING_TORCH_PROFILING=rate=0.01,backward=on`；
- 执行顺序为 baseline-1、profiled-1、profiled-2、baseline-2，以降低固定
  顺序造成的热机偏差；
- 使用总墙钟时间，包括进程启动、模型初始化和训练。

最初尝试了 TP=2、PP=2，但该作业卡在第一个 pipeline iteration，只有 NCCL
初始化阶段的 13 条 `coll_perf` 记录，因此该次尝试被排除。改为 PP=1 后训练正常，
能够持续输出 iteration，并在 8 个 rank 上生成 NCCL profiling 数据。

核心环境变量：

```bash
export CUDA_DEVICE_MAX_CONNECTIONS=1
export PROBING=2
export PROBING_TORCH_PROFILING="rate=0.01,backward=on"
export PROBING_MEGATRON=on
export PROBING_MEGATRON_STEP_SYNC=on
export PROBING_PORT=random
```

训练命令：

```bash
python3.12 -m torch.distributed.run \
  --standalone --nproc_per_node=8 \
  examples/megatron/megatron_mcore_train_loop.py \
  --tensor-model-parallel-size 2 \
  --pipeline-model-parallel-size 1 \
  --train-iters 200 \
  --print-freq 200 \
  --skip-checkpoint
```

profiled 组额外设置：

```bash
export NCCL_PROFILER_PLUGIN=libprobing_nccl_profiler.so
export NCCL_PROFILE_EVENT_MASK=94
export PROBING_NCCL_INFLIGHT_THRESHOLD_SECS=0
```

### 5.2 结果

| 执行顺序 | 模式 | 200 iterations 总时间 |
|---:|---|---:|
| 1 | Baseline | 96.965 s |
| 2 | Profiled | 117.782 s |
| 3 | Profiled | 79.695 s |
| 4 | Baseline | 114.853 s |

汇总：

```text
baseline 中位数 = 105.909 s
profiled 中位数 = 98.738 s
直接计算的中位变化 = -6.77%

第一组配对变化 = +21.47%
第二组配对变化 = -30.61%
```

两组配对一正一负，方向完全相反。`-6.77%` 不能解释为 profiler 使训练加速，
它反映的是短作业中的启动时间、GPU 频率变化、系统调度及随机 Torch step 采样噪声。
因此本轮 200-step Megatron 总墙钟实验不能识别出稳定的 NCCL profiler 开销。

profiled 两次运行均为 8 个 rank 生成了 `nccl.coll_perf`，每次对应的文件总大小约
32 MiB，说明插件确实处于工作状态。

## 6. 结论

本轮实验支持以下结论：

1. 在单节点 NVLink、event mask 94 下，16 MiB AllReduce 的稳健开销约为
   0.72%，处于 1% 左右。
2. 对 256 MiB AllReduce，除一次明显系统离群点外，没有观察到稳定性能损失。
3. 1 MiB 小消息的固定回调成本更容易被放大，但三次配对的波动范围达到
   -11.30% 到 +30.06%，尚不能给出可靠开销上界。
4. 200-step Megatron 总墙钟时间波动大于待测开销，不能用于得出 NCCL profiler
   的精确百分比。
5. `rate=0.01` 是 Torch profiling 的采样率，不会把 NCCL profiler 的回调率降到
   1%。

## 7. 后续实验建议

为了得到可用于性能准入的结果，建议：

1. 微基准提高到至少 2000 timed iterations 和 5–7 个 ABBA 配对；
2. 固定 GPU application clocks，并确保节点无其他 GPU/CPU/IO 负载；
3. Megatron 使用更长的稳态区间，单独记录 warmup 后的 per-step latency，
   不再使用包含初始化阶段的总墙钟时间；
4. 固定或记录每次 Torch sampled step 数，避免 `rate=0.01` 的随机采样差异污染
   NCCL A/B；
5. 在真实多节点任务中分别测试 mask 94 和 mask 222，覆盖 IB/RoCE、
   `nccl.proxy_ops` 和 NetPlugin 回调；
6. 同时检查 `nccl.profiler_counters` 中的 write error、pool exhausted 和事件数，
   确认低开销不是由于事件丢失造成的。

## 8. 实验产物

远端节点上的原始结果：

```text
/tmp/nccl_overhead_exp
/tmp/megatron_overhead_exp
```

这些路径位于 `/tmp`，节点释放或重启后可能丢失。需要长期保留时应复制到共享存储。

## 9. 纯 Torch 模块采样器稳态实验（8 卡缩放版）

### 9.1 与 32 卡参考实验的关系

参考实验使用 32 卡 Qwen3-8B、TP=2、PP=2、DP=8、mock 数据和 500 步。
当前可用节点只有 8 张 H200，因此本次保持模型、TP、PP、步数和采样配置不变，
仅把 DP 缩放为 2：

| 项目 | 本次配置 |
|---|---|
| 模型 | Qwen3-8B 架构，随机权重 |
| GPU | 8 × H200 |
| 并行 | TP=2、PP=2、DP=2 |
| 数据 | Megatron `MockGPTDataset` |
| 序列长度 | 64 |
| micro-batch | 1 |
| microbatches/step | 2 |
| 训练步数 | 500 |
| Torch 采样 | `random:0.01,backward=on,shadow=4:1` |
| NCCL native profiler | 未开启 |

本次对 `megatron_mcore_train_loop.py` 增加了 `qwen3-8b` preset。架构参数来自共享盘
Qwen3-8B 配置：36 层、hidden size 4096、32 attention heads、8 query groups、
FFN hidden size 12288、词表 151936、RMSNorm、SwiGLU 和 RoPE base 1000000。
训练使用 mock token，不加载预训练权重。

### 9.2 启动命令

```bash
cd /mnt/shared-storage-user/ailab-sys/zhangwensheng/probing

export PYTHONPATH="$PWD/python:/mnt/shared-storage-user/ailab-sys/zhangwensheng/pysite"
export LD_LIBRARY_PATH="/usr/local/nvidia/lib64:${LD_LIBRARY_PATH:-}"
export CUDA_DEVICE_MAX_CONNECTIONS=1

export PROBING=2
export PROBING_TORCH_PROFILING="random:0.01,backward=on,shadow=4:1"
export PROBING_MEGATRON=on
export PROBING_MEGATRON_STEP_SYNC=on
export PROBING_DATA_DIR=/dev/shm/qwen3_8b_torch_rate001_500
export PROBING_NCCL_MOCK=0

unset NCCL_PROFILER_PLUGIN
unset NCCL_PROFILE_EVENT_MASK

python3.12 -m torch.distributed.run \
  --standalone \
  --nproc_per_node=8 \
  --master_port=29720 \
  --local_addr=127.0.0.1 \
  examples/megatron/megatron_mcore_train_loop.py \
  --model-preset qwen3-8b \
  --tensor-model-parallel-size 2 \
  --pipeline-model-parallel-size 2 \
  --sequence-length 64 \
  --micro-batch-size 1 \
  --num-microbatches 2 \
  --train-iters 500 \
  --print-freq 10 \
  --hold-sec 900 \
  --skip-checkpoint
```

`--hold-sec` 只用于在训练结束后保持 probing socket 存活，以便查询 mmap 表，不计入
训练 step timing。

### 9.3 真值完整性

`python.torch_step_timing` 每卡均为 498 行，未触发环形上限：

| 类型 | 每卡行数 | 8 卡总行数 |
|---|---:|---:|
| normal | 394 | 3152 |
| shadow | 99 | 792 |
| sampled | 5 | 40 |
| 合计 | 498 | 3984 |

所有 rank 的 `sample_rate` 最小值和最大值均为 0.01。五个正式采样步固定为：

```text
2, 102, 202, 302, 402
```

`python.torch_trace` 共 73568 行：

- PP stage 0（rank 0–3）：每卡 9211 行；
- PP stage 1（rank 4–7）：每卡 9181 行；
- 每卡都低于 10000 行环形上限，因此没有覆盖丢失；
- rank 0–3 还包含初始化期间 `local_step=0` 的 1470 行，以及一个 step marker；
- 正式五个 sampled step 在 PP stage 0 每步各 1548 行，在 PP stage 1 每步各
  1836 行。

实验目录中 `nccl.*` 文件数为 0，确认没有加载 NCCL native profiler。

### 9.4 每卡结果

单位为 step duration 中位数（ms）。采样步开销定义为：

```text
sampled_penalty = (sampled_ms / normal_ms - 1) × 100%
```

实际摊薄开销使用本次真实采样占比 `5/498`：

```text
amortized = sampled_penalty × 5 / 498
```

| rank | normal ms | shadow ms | sampled ms | sampled 单步开销 | 实际摊薄开销 |
|---:|---:|---:|---:|---:|---:|
| 0 | 1087.66 | 1092.99 | 2571.44 | +136.42% | +1.37% |
| 1 | 1084.97 | 1103.65 | 2521.83 | +132.43% | +1.33% |
| 2 | 1087.90 | 1131.71 | 2560.99 | +135.41% | +1.36% |
| 3 | 1082.26 | 1117.12 | 2516.78 | +132.55% | +1.33% |
| 4 | 1147.25 | 1075.18 | 2247.72 | +95.92% | +0.96% |
| 5 | 1147.65 | 1074.80 | 2246.55 | +95.75% | +0.96% |
| 6 | 1112.83 | 1118.61 | 2801.68 | +151.76% | +1.52% |
| 7 | 1112.50 | 1117.76 | 2802.62 | +151.92% | +1.53% |

跨 rank 稳健汇总（rank 中位数）：

```text
normal step              1100.20 ms
shadow step              1110.38 ms
sampled step             2541.41 ms
shadow 相对 normal          +0.50%
sampled 单步开销           +133.98%
rate=0.01 实际摊薄开销       +1.35%
```

### 9.5 结论

1. 解析器正确接受 `random:0.01,backward=on,shadow=4:1`，表内实测
   `sample_rate=0.01`。
2. 每卡完整跑满 5 个采样周期，500 步已进入稳定摊薄区间。
3. 对 8 卡 Qwen3-8B TP2/PP2/DP2，完整逐模块 forward/backward 采样使被采样的
   单步中位耗时增加约 133.98%。
4. 由于每 100 步只采样一次，真实摊薄到全部 step 后的中位开销约为 1.35%。
5. shadow 与 normal 的 rank 中位差仅 0.50%，说明未采样步骤的 hook/调度基线
   接近正常步骤。
6. 该结果衡量的是纯 Torch 模块采样器，不是 NCCL profiler；不能用它替代
   NCCL 插件开/关 A/B 的结果。
7. 与 32 卡参考实验相比，本次 DP 从 8 缩小到 2，结论只适用于当前 8 卡缩放配置。

原始分析中间结果位于远端：

```text
/tmp/qwen3_8b_torch_rate001_500_analysis
/tmp/qwen3_8b_torch_rate001_500.log
```


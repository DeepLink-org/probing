# 性能分析示例

识别和修复 AI 工作负载中的性能瓶颈。

## 查找瓶颈

### 整体性能概况

```bash
probing $ENDPOINT query "
SELECT
    module,
    stage,
    COUNT(*) as executions,
    AVG(duration) as avg_time_sec,
    SUM(duration) as total_time_sec,
    SUM(duration) * 100.0 / SUM(SUM(duration)) OVER () as pct_time
FROM python.torch_trace
WHERE step > (SELECT MAX(step) - 10 FROM python.torch_trace)
GROUP BY module, stage
ORDER BY total_time_sec DESC
LIMIT 15"
```

### 每步分解

```bash
probing $ENDPOINT query "
SELECT
    step,
    SUM(CASE WHEN stage = 'forward' THEN duration ELSE 0 END) as forward_time,
    SUM(CASE WHEN stage = 'backward' THEN duration ELSE 0 END) as backward_time,
    SUM(CASE WHEN stage = 'step' THEN duration ELSE 0 END) as optimizer_time
FROM python.torch_trace
WHERE step > (SELECT MAX(step) - 5 FROM python.torch_trace)
GROUP BY step
ORDER BY step"
```

## GPU 利用率

### 检查当前利用率

```bash
probing $ENDPOINT eval "
import subprocess
result = subprocess.run(
    ['nvidia-smi', '--query-gpu=utilization.gpu,utilization.memory,temperature.gpu',
     '--format=csv,noheader,nounits'],
    capture_output=True, text=True
)
for i, line in enumerate(result.stdout.strip().split('\\n')):
    gpu_util, mem_util, temp = line.split(', ')
    print(f'GPU {i}: 利用率={gpu_util}%, 内存={mem_util}%, 温度={temp}°C')"
```

### CUDA 同步开销

```bash
probing $ENDPOINT eval "
import torch
import time

# 测量同步开销
start = time.perf_counter()
torch.cuda.synchronize()
sync_time = time.perf_counter() - start
print(f'CUDA 同步时间: {sync_time*1000:.2f} ms')"
```

## 内存带宽

### 内存受限操作

```bash
probing $ENDPOINT query "
SELECT
    module,
    AVG(allocated) as avg_memory_mb,
    AVG(duration) as avg_time_sec,
    AVG(allocated) / AVG(duration) as memory_bandwidth_mb_per_sec
FROM python.torch_trace
WHERE duration > 0.001
GROUP BY module
ORDER BY memory_bandwidth_mb_per_sec DESC
LIMIT 10"
```

## 数据加载性能

### 数据加载器计时

```bash
probing $ENDPOINT eval "
import time

# 计时一个批次加载
start = time.perf_counter()
batch = next(iter(train_loader))
load_time = time.perf_counter() - start
print(f'批次加载时间: {load_time*1000:.2f} ms')"
```

### Worker 分析

```bash
probing $ENDPOINT eval "
print(f'Num workers: {train_loader.num_workers}')
print(f'Pin memory: {train_loader.pin_memory}')
print(f'Prefetch factor: {getattr(train_loader, \"prefetch_factor\", 2)}')"
```

## 通信开销（分布式）

### NCCL 操作时间

```bash
probing $ENDPOINT query "
SELECT
    operation_type,
    COUNT(*) as count,
    AVG(duration_ms) as avg_time_ms,
    MAX(duration_ms) as max_time_ms
FROM python.nccl_trace
GROUP BY operation_type
ORDER BY avg_time_ms DESC"
```

### All-Reduce 扩展

```bash
probing $ENDPOINT eval "
import torch.distributed as dist
import time

if dist.is_initialized():
    tensor = torch.randn(1000000, device='cuda')

    start = time.perf_counter()
    dist.all_reduce(tensor)
    torch.cuda.synchronize()
    allreduce_time = time.perf_counter() - start

    print(f'4MB All-reduce 时间: {allreduce_time*1000:.2f} ms')"
```

## Attention 瓶颈

### Self-Attention 分析

```bash
probing $ENDPOINT query "
SELECT
    module,
    AVG(duration) as avg_time,
    AVG(allocated) as avg_memory
FROM python.torch_trace
WHERE module LIKE '%attention%' OR module LIKE '%attn%'
GROUP BY module
ORDER BY avg_time DESC"
```

### 每序列长度的内存

```bash
probing $ENDPOINT eval "
import torch

# 检查 attention 内存扩展
seq_len = model.config.max_position_embeddings
hidden = model.config.hidden_size
num_heads = model.config.num_attention_heads

# Attention 分数内存: O(seq_len^2)
attention_memory = seq_len * seq_len * num_heads * 4 / 1024**3  # GB
print(f'估计 attention 内存: {attention_memory:.2f} GB')"
```

## 优化建议

### 基于分析的优化

```bash
# 1. 识别最慢模块
probing $ENDPOINT query "
SELECT module, AVG(duration) as avg_time
FROM python.torch_trace
GROUP BY module
ORDER BY avg_time DESC
LIMIT 5"

# 2. 检查是计算受限还是内存受限
probing $ENDPOINT eval "
import torch
# 高计算利用率 + 低内存带宽 = 计算受限
# 低计算利用率 + 高内存利用率 = 内存受限"
```

### 常见优化

#### 启用 Torch Compile

```bash
probing $ENDPOINT eval "
import torch
if hasattr(torch, 'compile'):
    model = torch.compile(model)
    print('模型已用 torch.compile 编译')"
```

#### 启用混合精度

```bash
probing $ENDPOINT eval "
from torch.cuda.amp import autocast
print(f'AMP 已启用: {torch.cuda.amp.autocast_mode.is_autocast_enabled()}')"
```

#### 检查梯度检查点

```bash
probing $ENDPOINT eval "
# 检查是否启用梯度检查点
for name, module in model.named_modules():
    if hasattr(module, 'gradient_checkpointing'):
        print(f'{name}: checkpoint={module.gradient_checkpointing}')"
```

## 基准测试

### 吞吐量测量

```bash
probing $ENDPOINT eval "
import time

# 测量 10 步的吞吐量
steps = 10
start = time.perf_counter()
for _ in range(steps):
    trainer.train_step()
elapsed = time.perf_counter() - start

samples_per_sec = (steps * batch_size) / elapsed
print(f'吞吐量: {samples_per_sec:.1f} samples/sec')"
```

### 优化前后对比

```bash
# 优化前
probing $ENDPOINT query "
SELECT AVG(duration) as before_avg
FROM python.torch_trace
WHERE step BETWEEN 100 AND 110"

# 优化后
probing $ENDPOINT query "
SELECT AVG(duration) as after_avg
FROM python.torch_trace
WHERE step BETWEEN 200 AND 210"
```

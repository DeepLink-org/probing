# Performance Analysis Examples

Identify and fix performance bottlenecks in AI workloads.

## Finding Bottlenecks

### Overall Performance Profile

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

### Per-Step Breakdown

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

## GPU Utilization

### Check Current Utilization

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
    print(f'GPU {i}: Util={gpu_util}%, Mem={mem_util}%, Temp={temp}°C')"
```

### CUDA Synchronization Overhead

```bash
probing $ENDPOINT eval "
import torch
import time

# Measure sync overhead
start = time.perf_counter()
torch.cuda.synchronize()
sync_time = time.perf_counter() - start
print(f'CUDA sync time: {sync_time*1000:.2f} ms')"
```

## Memory Bandwidth

### Memory-Bound Operations

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

## Data Loading Performance

### Data Loader Timing

```bash
probing $ENDPOINT eval "
import time

# Time one batch load
start = time.perf_counter()
batch = next(iter(train_loader))
load_time = time.perf_counter() - start
print(f'Batch load time: {load_time*1000:.2f} ms')"
```

### Worker Analysis

```bash
probing $ENDPOINT eval "
print(f'Num workers: {train_loader.num_workers}')
print(f'Pin memory: {train_loader.pin_memory}')
print(f'Prefetch factor: {getattr(train_loader, \"prefetch_factor\", 2)}')"
```

## Communication Overhead (Distributed)

### NCCL Operation Times

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

### All-Reduce Scaling

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

    print(f'All-reduce time for 4MB: {allreduce_time*1000:.2f} ms')"
```

## Attention Bottlenecks

### Self-Attention Analysis

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

### Memory per Sequence Length

```bash
probing $ENDPOINT eval "
import torch

# Check attention memory scaling
seq_len = model.config.max_position_embeddings
hidden = model.config.hidden_size
num_heads = model.config.num_attention_heads

# Attention score memory: O(seq_len^2)
attention_memory = seq_len * seq_len * num_heads * 4 / 1024**3  # GB
print(f'Estimated attention memory: {attention_memory:.2f} GB')"
```

## Optimization Recommendations

### Profile-Guided Optimization

```bash
# 1. Identify slowest modules
probing $ENDPOINT query "
SELECT module, AVG(duration) as avg_time
FROM python.torch_trace
GROUP BY module
ORDER BY avg_time DESC
LIMIT 5"

# 2. Check if compute-bound or memory-bound
probing $ENDPOINT eval "
import torch
# High compute utilization + low memory bandwidth = compute-bound
# Low compute utilization + high memory utilization = memory-bound"
```

### Common Optimizations

#### Enable Torch Compile

```bash
probing $ENDPOINT eval "
import torch
if hasattr(torch, 'compile'):
    model = torch.compile(model)
    print('Model compiled with torch.compile')"
```

#### Enable Mixed Precision

```bash
probing $ENDPOINT eval "
from torch.cuda.amp import autocast
print(f'AMP enabled: {torch.cuda.amp.autocast_mode.is_autocast_enabled()}')"
```

#### Check Gradient Checkpointing

```bash
probing $ENDPOINT eval "
# Check if gradient checkpointing is enabled
for name, module in model.named_modules():
    if hasattr(module, 'gradient_checkpointing'):
        print(f'{name}: checkpoint={module.gradient_checkpointing}')"
```

## Benchmarking

### Throughput Measurement

```bash
probing $ENDPOINT eval "
import time

# Measure throughput over 10 steps
steps = 10
start = time.perf_counter()
for _ in range(steps):
    trainer.train_step()
elapsed = time.perf_counter() - start

samples_per_sec = (steps * batch_size) / elapsed
print(f'Throughput: {samples_per_sec:.1f} samples/sec')"
```

### Compare Before/After

```bash
# Before optimization
probing $ENDPOINT query "
SELECT AVG(duration) as before_avg
FROM python.torch_trace
WHERE step BETWEEN 100 AND 110"

# After optimization
probing $ENDPOINT query "
SELECT AVG(duration) as after_avg
FROM python.torch_trace
WHERE step BETWEEN 200 AND 210"
```

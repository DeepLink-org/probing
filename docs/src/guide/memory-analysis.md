# Memory Analysis

Probing provides comprehensive tools for analyzing memory usage in AI applications.

## Overview

Memory issues are common in AI workloads, especially during training. Probing helps you:

- Track GPU and CPU memory allocation
- Detect memory leaks
- Analyze memory usage patterns
- Optimize memory efficiency

## Quick Memory Check

```bash
# Get current memory state
probing $ENDPOINT eval "
import torch
import psutil

proc = psutil.Process()
print(f'CPU Memory: {proc.memory_info().rss / 1024**3:.2f} GB')

if torch.cuda.is_available():
    print(f'GPU Allocated: {torch.cuda.memory_allocated() / 1024**3:.2f} GB')
    print(f'GPU Cached: {torch.cuda.memory_reserved() / 1024**3:.2f} GB')
"
```

## Memory Usage Trends

### Track Memory Over Training Steps

```sql
SELECT
  step,
  avg(allocated) as avg_memory_mb,
  max(allocated) as peak_memory_mb,
  min(allocated) as min_memory_mb
FROM python.torch_trace
WHERE step IS NOT NULL
GROUP BY step
ORDER BY step;
```

### Detect Memory Growth

```sql
SELECT
  step,
  max(allocated) - min(allocated) as memory_growth_mb
FROM python.torch_trace
WHERE step > (SELECT max(step) - 10 FROM python.torch_trace)
GROUP BY step
HAVING max(allocated) - min(allocated) > 50
ORDER BY step;
```

## Memory by Module

Identify which model components use the most memory:

```sql
SELECT
  module,
  stage,
  avg(allocated) as avg_memory,
  count(*) as execution_count
FROM python.torch_trace
WHERE module IS NOT NULL
GROUP BY module, stage
ORDER BY avg_memory DESC
LIMIT 10;
```

## Memory Leak Detection

### Pattern: Monotonic Growth

```sql
WITH memory_trend AS (
  SELECT
    step,
    max(allocated) as peak_memory,
    LAG(max(allocated)) OVER (ORDER BY step) as prev_peak
  FROM python.torch_trace
  GROUP BY step
)
SELECT step, peak_memory, prev_peak,
       peak_memory - prev_peak as growth
FROM memory_trend
WHERE peak_memory > prev_peak
ORDER BY step;
```

### Force Garbage Collection

```bash
probing $ENDPOINT eval "
import gc
import torch

# Force garbage collection
gc.collect()

# Clear CUDA cache
if torch.cuda.is_available():
    torch.cuda.empty_cache()
    print('CUDA cache cleared')

print(f'Garbage collected: {gc.get_count()}')
"
```

## GPU Memory Analysis

### Current GPU State

```bash
probing $ENDPOINT eval "
import torch

if torch.cuda.is_available():
    for i in range(torch.cuda.device_count()):
        props = torch.cuda.get_device_properties(i)
        allocated = torch.cuda.memory_allocated(i) / 1024**3
        reserved = torch.cuda.memory_reserved(i) / 1024**3
        total = props.total_memory / 1024**3

        print(f'GPU {i}: {props.name}')
        print(f'  Total: {total:.2f} GB')
        print(f'  Allocated: {allocated:.2f} GB')
        print(f'  Reserved: {reserved:.2f} GB')
        print(f'  Free: {total - reserved:.2f} GB')
"
```

### Memory by Operation Stage

```sql
SELECT
  stage,
  avg(allocated) as avg_allocated,
  avg(max_allocated) as avg_peak,
  avg(cached) as avg_cached
FROM python.torch_trace
WHERE step = (SELECT max(step) FROM python.torch_trace)
GROUP BY stage
ORDER BY avg_peak DESC;
```

## Best Practices

### 1. Regular Memory Snapshots

Take periodic snapshots during training:

```bash
# Add to training loop
probing $ENDPOINT eval "
import torch
step = ...  # current step
if step % 100 == 0:
    allocated = torch.cuda.memory_allocated() / 1024**3
    print(f'Step {step}: {allocated:.2f} GB')
"
```

### 2. Profile Memory-Intensive Operations

```sql
-- Find operations with highest memory variance
SELECT
  module,
  stage,
  stddev(allocated) as memory_variance,
  max(allocated) - min(allocated) as memory_range
FROM python.torch_trace
GROUP BY module, stage
HAVING stddev(allocated) > 10
ORDER BY memory_variance DESC;
```

### 3. Monitor Memory Fragmentation

```bash
probing $ENDPOINT eval "
import torch
if torch.cuda.is_available():
    stats = torch.cuda.memory_stats()
    print(f'Allocated blocks: {stats.get(\"allocated_bytes.all.current\", 0) / 1024**3:.2f} GB')
    print(f'Reserved blocks: {stats.get(\"reserved_bytes.all.current\", 0) / 1024**3:.2f} GB')
"
```

## Troubleshooting

### Out of Memory (OOM)

1. Check current memory state
2. Identify memory-heavy modules
3. Force garbage collection
4. Reduce batch size or model size

### Memory Not Released

1. Check for circular references
2. Verify tensors are not held in lists/dicts
3. Use `del` explicitly for large tensors
4. Call `torch.cuda.empty_cache()`

## Next Steps

- [SQL Analytics](sql-analytics.md) - More query patterns
- [Debugging](debugging.md) - Stack analysis techniques
- [Troubleshooting](troubleshooting.md) - Common issues

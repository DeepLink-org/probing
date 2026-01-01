# Memory Leak Examples

Detecting and fixing memory leaks in AI applications.

## Identifying Memory Leaks

### Memory Growth Pattern

```bash
# Track memory over steps
probing $ENDPOINT query "
SELECT
    step,
    MAX(allocated) as peak_memory_mb
FROM python.torch_trace
GROUP BY step
ORDER BY step"
```

### Monotonic Growth Detection

```sql
WITH memory_trend AS (
  SELECT
    step,
    MAX(allocated) as peak,
    LAG(MAX(allocated)) OVER (ORDER BY step) as prev_peak
  FROM python.torch_trace
  GROUP BY step
)
SELECT
    step,
    peak,
    peak - prev_peak as growth
FROM memory_trend
WHERE peak > prev_peak
ORDER BY step;
```

## Common Leak Patterns

### Pattern 1: Accumulating Tensors in Lists

**Problem:**

```python
# BAD: Tensors accumulate in list
all_losses = []
for batch in dataloader:
    loss = model(batch)
    all_losses.append(loss)  # Holds computation graph!
```

**Detection:**

```bash
probing $ENDPOINT eval "
import gc
import torch
tensors = [obj for obj in gc.get_objects() if isinstance(obj, torch.Tensor)]
print(f'Total tensors: {len(tensors)}')"
```

**Fix:**

```bash
probing $ENDPOINT eval "
# Use .item() to extract scalar
all_losses.clear()
print('Cleared loss list')"
```

### Pattern 2: Forgotten Gradient Graphs

**Problem:**

```python
# BAD: Intermediate tensors hold grad_fn
intermediate = model.encoder(x)
# ... lots of operations ...
# intermediate still holds computation graph
```

**Detection:**

```bash
probing $ENDPOINT eval "
import torch
for name, param in model.named_parameters():
    if param.grad is not None:
        print(f'{name}: grad_fn={param.grad.grad_fn}')"
```

**Fix:**

```bash
probing $ENDPOINT eval "
model.zero_grad(set_to_none=True)
import torch
torch.cuda.empty_cache()
print('Cleared gradients')"
```

### Pattern 3: Circular References

**Detection:**

```bash
probing $ENDPOINT eval "
import gc
gc.set_debug(gc.DEBUG_SAVEALL)
gc.collect()
print(f'Uncollectable: {len(gc.garbage)}')"
```

## GPU Memory Leaks

### Check CUDA Memory State

```bash
probing $ENDPOINT eval "
import torch
print(f'Allocated: {torch.cuda.memory_allocated() / 1024**3:.2f} GB')
print(f'Reserved: {torch.cuda.memory_reserved() / 1024**3:.2f} GB')
print(f'Max allocated: {torch.cuda.max_memory_allocated() / 1024**3:.2f} GB')"
```

### Memory Snapshot

```bash
probing $ENDPOINT eval "
import torch
if torch.cuda.is_available():
    snapshot = torch.cuda.memory_snapshot()
    print(f'Number of blocks: {len(snapshot)}')"
```

### Force CUDA Cleanup

```bash
probing $ENDPOINT eval "
import torch
import gc

# Clear all references
gc.collect()

# Empty CUDA cache
torch.cuda.empty_cache()

# Reset peak stats
torch.cuda.reset_peak_memory_stats()

print('CUDA memory cleaned')"
```

## CPU Memory Leaks

### Track Process Memory

```bash
probing $ENDPOINT eval "
import psutil
import os

proc = psutil.Process(os.getpid())
mem = proc.memory_info()
print(f'RSS: {mem.rss / 1024**3:.2f} GB')
print(f'VMS: {mem.vms / 1024**3:.2f} GB')"
```

### Find Large Objects

```bash
probing $ENDPOINT eval "
import sys
import gc

# Find largest objects
objects = gc.get_objects()
sizes = [(sys.getsizeof(obj), type(obj).__name__) for obj in objects[:1000]]
sizes.sort(reverse=True)
for size, name in sizes[:10]:
    print(f'{name}: {size / 1024:.1f} KB')"
```

## Data Loader Leaks

### Check Worker State

```bash
probing $ENDPOINT eval "
print(f'Num workers: {train_loader.num_workers}')
print(f'Batch size: {train_loader.batch_size}')"
```

### Inspect Prefetched Data

```bash
probing $ENDPOINT eval "
# Check if data is being held
if hasattr(train_loader, '_iterator'):
    print('Iterator exists')
else:
    print('No active iterator')"
```

## Monitoring Over Time

### Periodic Memory Check

```bash
# Run every minute
while true; do
    probing $ENDPOINT eval "
import torch
import psutil
import os
proc = psutil.Process(os.getpid())
print(f'CPU: {proc.memory_info().rss / 1024**3:.2f} GB, GPU: {torch.cuda.memory_allocated() / 1024**3:.2f} GB')"
    sleep 60
done
```

### SQL-Based Monitoring

```sql
-- Memory trend over last 100 steps
SELECT
    step,
    AVG(allocated) as avg_memory,
    MAX(allocated) as peak_memory
FROM python.torch_trace
WHERE step > (SELECT MAX(step) - 100 FROM python.torch_trace)
GROUP BY step
ORDER BY step;
```

## Prevention Best Practices

### 1. Use Context Managers

```python
# Good practice
with torch.no_grad():
    output = model(input)
```

### 2. Detach Tensors

```python
# When storing for later
stored_output = output.detach().cpu()
```

### 3. Clear Caches Periodically

```python
if step % 100 == 0:
    gc.collect()
    torch.cuda.empty_cache()
```

### 4. Use `set_to_none=True`

```python
optimizer.zero_grad(set_to_none=True)  # More efficient
```

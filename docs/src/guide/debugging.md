# Debugging Guide

Master the art of debugging AI applications with Probing's powerful introspection capabilities.

## Overview

Probing provides three complementary debugging approaches:

- **backtrace**: Capture execution context with stack frames
- **eval**: Execute arbitrary Python code in the target process
- **query**: Analyze collected data with SQL

## Stack Analysis

### Capture Current Stack

```bash
# Get current execution stack
probing $ENDPOINT backtrace

# Query stack frames
probing $ENDPOINT query "
SELECT func, file, lineno, depth
FROM python.backtrace
ORDER BY depth
LIMIT 10"
```

### Understanding Stack Frames

The `python.backtrace` table provides:

| Column | Description |
|--------|-------------|
| `func` | Function name |
| `file` | Source file path |
| `lineno` | Line number |
| `depth` | Stack depth (0 = deepest) |
| `frame_type` | 'Python' or 'Native' |

### Find Where Code Is Stuck

```bash
# Capture stack
probing $ENDPOINT backtrace

# Check top of stack
probing $ENDPOINT query "
SELECT func, file, lineno
FROM python.backtrace
WHERE depth = 0"
```

## Live Inspection

### Inspect Variables

```bash
# Check global variables
probing $ENDPOINT eval "print(globals().keys())"

# Inspect specific object
probing $ENDPOINT eval "print(type(model), model)"

# Check model parameters
probing $ENDPOINT eval "
for name, param in model.named_parameters():
    print(f'{name}: {param.shape}, grad={param.grad is not None}')
"
```

### Check Thread State

```bash
probing $ENDPOINT eval "
import threading
for t in threading.enumerate():
    print(f'{t.name}: alive={t.is_alive()}, daemon={t.daemon}')
"
```

### Monitor Training Progress

```bash
probing $ENDPOINT eval "
# Check current step
print(f'Current step: {trainer.current_step}')
print(f'Current loss: {trainer.last_loss}')
print(f'Learning rate: {optimizer.param_groups[0][\"lr\"]}')
"
```

## Debugging Scenarios

### Scenario 1: Training Hangs

**Symptoms**: Training progress stops, no errors.

**Diagnosis**:

```bash
# 1. Capture stack
probing $ENDPOINT backtrace

# 2. Check what's blocking
probing $ENDPOINT query "
SELECT func, file, lineno
FROM python.backtrace
WHERE depth < 5"

# 3. Check thread states
probing $ENDPOINT eval "
import threading
for t in threading.enumerate():
    print(f'{t.name}: {t.is_alive()}')"

# 4. Check for deadlocks
probing $ENDPOINT eval "
import torch.distributed as dist
if dist.is_initialized():
    print(f'Rank: {dist.get_rank()}, World: {dist.get_world_size()}')"
```

### Scenario 2: NaN/Inf in Gradients

**Symptoms**: Loss becomes NaN or Inf.

**Diagnosis**:

```bash
# Check for NaN in model parameters
probing $ENDPOINT eval "
import torch
for name, param in model.named_parameters():
    if param.grad is not None:
        if torch.isnan(param.grad).any():
            print(f'NaN gradient in {name}')
        if torch.isinf(param.grad).any():
            print(f'Inf gradient in {name}')"

# Check loss value
probing $ENDPOINT eval "
import torch
print(f'Loss: {loss.item()}')
print(f'IsNaN: {torch.isnan(loss).item()}')
print(f'IsInf: {torch.isinf(loss).item()}')"
```

### Scenario 3: Slow Training

**Symptoms**: Training slower than expected.

**Diagnosis**:

```sql
-- Find slowest operations
SELECT module, stage, avg(duration) as avg_time
FROM python.torch_trace
WHERE step > (SELECT max(step) - 5 FROM python.torch_trace)
GROUP BY module, stage
ORDER BY avg_time DESC
LIMIT 10;
```

```bash
# Check GPU utilization
probing $ENDPOINT eval "
import torch
if torch.cuda.is_available():
    print(f'CUDA synchronize time test...')
    import time
    start = time.time()
    torch.cuda.synchronize()
    print(f'Sync took: {time.time() - start:.3f}s')"
```

## Advanced Debugging

### Conditional Breakpoints (Conceptual)

```bash
# Monitor until condition is met
probing $ENDPOINT eval "
import torch
step = trainer.current_step
loss = trainer.last_loss
if loss > 10 or torch.isnan(torch.tensor(loss)):
    print(f'ALERT: Step {step}, Loss {loss}')
    # Trigger investigation
"
```

### Data Pipeline Debugging

```bash
# Check data loader
probing $ENDPOINT eval "
batch = next(iter(train_loader))
print(f'Batch shape: {batch[0].shape}')
print(f'Batch dtype: {batch[0].dtype}')
print(f'Has NaN: {torch.isnan(batch[0]).any()}')"
```

### Distributed Debugging

```bash
# Check distributed state
probing $ENDPOINT eval "
import torch.distributed as dist
if dist.is_initialized():
    print(f'Backend: {dist.get_backend()}')
    print(f'Rank: {dist.get_rank()}')
    print(f'World size: {dist.get_world_size()}')
    print(f'Is NCCL available: {dist.is_nccl_available()}')"
```

## Best Practices

1. **Start with backtrace** - Understand where execution is before diving deeper
2. **Use query for trends** - SQL is great for analyzing patterns over time
3. **Use eval for real-time** - Get current state with Python code
4. **Combine approaches** - backtrace → eval → query workflow
5. **Log important states** - Use eval to print diagnostic information

## Next Steps

- [Memory Analysis](memory-analysis.md) - Debug memory issues
- [Troubleshooting](troubleshooting.md) - Common problems and solutions
- [Design Architecture](../design/architecture.md) - Understand internals

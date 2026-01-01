# Training Debugging Examples

Common training debugging scenarios and solutions.

## Training Hangs

### Symptoms

- Loss stops updating
- No error messages
- Process still running

### Diagnosis

```bash
# 1. Capture stack trace
probing $ENDPOINT backtrace

# 2. Check where execution is stuck
probing $ENDPOINT query "
SELECT func, file, lineno, depth
FROM python.backtrace
ORDER BY depth
LIMIT 10"

# 3. Check thread states
probing $ENDPOINT eval "
import threading
for t in threading.enumerate():
    print(f'{t.name}: alive={t.is_alive()}')"
```

### Common Causes

#### Data Loader Stall

```bash
# Check data loader
probing $ENDPOINT eval "
import torch.utils.data
print(f'DataLoader workers: {train_loader.num_workers}')
print(f'Prefetch factor: {train_loader.prefetch_factor}')"
```

#### Distributed Deadlock

```bash
# Check distributed state
probing $ENDPOINT eval "
import torch.distributed as dist
if dist.is_initialized():
    print(f'Rank: {dist.get_rank()}')
    print(f'Waiting for barrier...')
    # Don't actually call barrier here!"
```

## Loss Explosion

### Symptoms

- Loss becomes NaN or Inf
- Training diverges

### Diagnosis

```bash
# Check for NaN in gradients
probing $ENDPOINT eval "
import torch
for name, param in model.named_parameters():
    if param.grad is not None:
        has_nan = torch.isnan(param.grad).any().item()
        has_inf = torch.isinf(param.grad).any().item()
        if has_nan or has_inf:
            print(f'{name}: NaN={has_nan}, Inf={has_inf}')"
```

### Fix: Gradient Clipping

```bash
# Check current clipping
probing $ENDPOINT eval "
print(f'Grad clip value: {getattr(trainer, \"grad_clip\", None)}')"

# Apply emergency clipping
probing $ENDPOINT eval "
torch.nn.utils.clip_grad_norm_(model.parameters(), 1.0)"
```

## Slow Training

### Diagnosis

```bash
# Find slowest modules
probing $ENDPOINT query "
SELECT
    module,
    stage,
    COUNT(*) as count,
    AVG(duration) as avg_time,
    SUM(duration) as total_time
FROM python.torch_trace
WHERE step > (SELECT MAX(step) - 5 FROM python.torch_trace)
GROUP BY module, stage
ORDER BY total_time DESC
LIMIT 10"
```

### Check GPU Utilization

```bash
probing $ENDPOINT eval "
import subprocess
result = subprocess.run(['nvidia-smi', '--query-gpu=utilization.gpu', '--format=csv,noheader'],
                        capture_output=True, text=True)
print(f'GPU Utilization: {result.stdout.strip()}')"
```

## Memory Issues During Training

### Track Memory Growth

```bash
probing $ENDPOINT query "
SELECT
    step,
    MAX(allocated) as peak_memory,
    MAX(allocated) - MIN(allocated) as memory_range
FROM python.torch_trace
WHERE step > (SELECT MAX(step) - 20 FROM python.torch_trace)
GROUP BY step
ORDER BY step"
```

### Force Cleanup

```bash
probing $ENDPOINT eval "
import gc
import torch

# Clear gradients
model.zero_grad(set_to_none=True)

# Garbage collection
gc.collect()

# Clear CUDA cache
if torch.cuda.is_available():
    torch.cuda.empty_cache()

print('Cleanup complete')"
```

## Checkpoint Recovery

### Save Emergency Checkpoint

```bash
probing $ENDPOINT eval "
import torch
checkpoint = {
    'step': trainer.current_step,
    'model': model.state_dict(),
    'optimizer': optimizer.state_dict(),
}
torch.save(checkpoint, 'emergency_checkpoint.pt')
print('Emergency checkpoint saved')"
```

### Inspect Checkpoint

```bash
probing $ENDPOINT eval "
import torch
ckpt = torch.load('checkpoint.pt', map_location='cpu')
print(f'Keys: {ckpt.keys()}')
print(f'Step: {ckpt.get(\"step\", \"N/A\")}')"
```

## Learning Rate Issues

### Check Current LR

```bash
probing $ENDPOINT eval "
for i, pg in enumerate(optimizer.param_groups):
    print(f'Group {i}: lr={pg[\"lr\"]}, weight_decay={pg.get(\"weight_decay\", 0)}')"
```

### Adjust LR

```bash
# Reduce learning rate
probing $ENDPOINT eval "
for pg in optimizer.param_groups:
    pg['lr'] *= 0.1
print(f'New LR: {optimizer.param_groups[0][\"lr\"]}')"
```

## Distributed Training Issues

### Check All Ranks

```bash
# Run on each node
probing -t node1:8080 eval "print(f'Rank 0 step: {trainer.current_step}')"
probing -t node2:8080 eval "print(f'Rank 1 step: {trainer.current_step}')"
```

### Verify Synchronization

```bash
probing $ENDPOINT eval "
import torch.distributed as dist
if dist.is_initialized():
    tensor = torch.tensor([trainer.current_step], device='cuda')
    dist.all_reduce(tensor)
    print(f'Sum of steps across ranks: {tensor.item()}')"
```

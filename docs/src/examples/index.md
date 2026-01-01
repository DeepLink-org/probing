# Examples

Real-world examples demonstrating Probing's capabilities.

## Overview

These examples show common debugging and profiling scenarios in AI/ML workflows.

| Example | Description |
|---------|-------------|
| [Training Debugging](training-debugging.md) | Debug training issues |
| [Memory Leak](memory-leak.md) | Find and fix memory leaks |
| [Performance Analysis](performance-analysis.md) | Identify bottlenecks |

## Quick Examples

### Check Training Progress

```bash
probing $ENDPOINT eval "
print(f'Step: {trainer.current_step}')
print(f'Loss: {trainer.last_loss:.4f}')
print(f'LR: {optimizer.param_groups[0][\"lr\"]:.6f}')"
```

### Monitor GPU Memory

```bash
probing $ENDPOINT eval "
import torch
allocated = torch.cuda.memory_allocated() / 1024**3
reserved = torch.cuda.memory_reserved() / 1024**3
print(f'Allocated: {allocated:.2f} GB')
print(f'Reserved: {reserved:.2f} GB')"
```

### Find Slow Operations

```bash
probing $ENDPOINT query "
SELECT module, AVG(duration) as avg_time
FROM python.torch_trace
WHERE step > (SELECT MAX(step) - 5 FROM python.torch_trace)
GROUP BY module
ORDER BY avg_time DESC
LIMIT 5"
```

### Check Thread States

```bash
probing $ENDPOINT eval "
import threading
for t in threading.enumerate():
    print(f'{t.name}: alive={t.is_alive()}, daemon={t.daemon}')"
```

## Running Examples

All examples assume you have:

1. A running Python process with Probing enabled
2. The `$ENDPOINT` environment variable set

```bash
# Set endpoint
export ENDPOINT=12345  # Process ID
# or
export ENDPOINT=host:8080  # Remote address

# Run example commands
probing $ENDPOINT eval "..."
```

## Contributing Examples

Have a useful debugging pattern? Contributions welcome!

1. Fork the repository
2. Add your example to `docs/src/examples/`
3. Submit a pull request

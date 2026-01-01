# Why Probing?

Probing is a dynamic performance analysis tool designed specifically for AI applications. This document details Probing's core technical advantages and design philosophy.

## Design Philosophy

### Zero Intrusion Principle

Traditional profiling tools typically require code modifications:

```python
# ❌ Traditional approach: requires code changes
import logging
import time

def train_step(model, data):
    start = time.time()
    logging.info("Starting train step")

    loss = model(data)

    logging.info(f"Train step took {time.time() - start:.3f}s")
    return loss
```

Probing takes a completely different approach:

```bash
# ✅ Probing approach: zero code changes
probing -t <pid> inject
probing -t <pid> query "SELECT * FROM python.torch_trace"
```

### SQL-Driven Analysis

Why SQL instead of fixed reports?

| Fixed Reports | SQL Queries |
|---------------|-------------|
| Predefined formats | Flexible custom queries |
| Export then process | Real-time interactive analysis |
| Hard to drill down | Aggregate on any dimension |
| Learn proprietary syntax | Universal SQL skills |

```sql
-- Example: Find the 10 most time-consuming operations
SELECT
    operation_name,
    AVG(duration_ms) as avg_duration,
    COUNT(*) as call_count
FROM python.torch_trace
WHERE timestamp > now() - interval '5 minutes'
GROUP BY operation_name
ORDER BY avg_duration DESC
LIMIT 10
```

## Core Technical Advantages

### 1. Dynamic Probe Injection

#### Technical Implementation

Probing uses the Linux ptrace system call for code injection:

```
Target Process                      Probing CLI
    │                                  │
    ▼                                  ▼
┌─────────┐    1. ptrace attach    ┌─────────┐
│ Running │ ◄──────────────────────│ Tracer  │
│ Process │                        │         │
└─────────┘                        └─────────┘
    │                                  │
    ▼                                  ▼
┌─────────┐    2. Inject shellcode ┌─────────┐
│ Paused  │ ◄──────────────────────│ Inject  │
│         │                        │         │
└─────────┘                        └─────────┘
    │                                  │
    ▼                                  ▼
┌─────────┐    3. Call dlopen      ┌─────────┐
│ Load    │ ◄──────────────────────│ Execute │
│ Library │                        │         │
└─────────┘                        └─────────┘
    │                                  │
    ▼                                  ▼
┌─────────┐    4. Resume execution ┌─────────┐
│ Resume  │ ◄──────────────────────│ Detach  │
│ + Probe │                        │         │
└─────────┘                        └─────────┘
```

#### Safety Guarantees

- **Complete state preservation**: Save all registers and overwritten memory before injection
- **Atomic operations**: Full rollback on injection failure
- **Permission checks**: Only process owner can inject
- **Memory alignment**: Ensure 16-byte stack pointer alignment (x86-64 ABI)

### 2. DataFusion-Based Query Engine

#### Why DataFusion?

| Feature | DataFusion | Custom Engine |
|---------|------------|---------------|
| Development cost | Low | High |
| SQL compatibility | Complete | Partial |
| Performance optimization | Mature | Needs accumulation |
| Community support | Active | None |
| Arrow integration | Native | Needs adaptation |

#### Plugin Architecture

```rust
/// Plugin trait definition
pub trait Plugin {
    fn name(&self) -> String;
    fn kind(&self) -> PluginType;
    fn namespace(&self) -> String;
    fn register_table(&self, ...) -> Result<()>;
}
```

Built-in plugins:

- `python.backtrace` - Python call stack
- `python.torch_trace` - PyTorch operation tracing
- `python.memory` - Memory usage statistics
- `system.process` - Process information

### 3. Remote REPL

#### How It Works

```
┌──────────────┐         ┌──────────────┐
│   CLI REPL   │  HTTP   │ Target Process│
│              │ ─────► │              │
│ >>> expr     │         │  Python      │
│              │ ◄───── │  Interpreter │
│ result       │  JSON   │  Exec+Return │
└──────────────┘         └──────────────┘
```

#### Use Cases

```python
# Connect to running process
probing -t <pid> repl

>>> # Inspect model parameters
>>> model = [m for m in gc.get_objects() if isinstance(m, torch.nn.Module)][0]
>>> print(sum(p.numel() for p in model.parameters()))
125000000

>>> # Check GPU memory
>>> print(torch.cuda.memory_allocated() / 1e9, "GB")
12.5 GB

>>> # Check current loss
>>> print(loss.item())
0.0234
```

### 4. Multi-Version Python Support

Probing supports all Python versions from 3.4 to 3.13:

```
Python Version │  Frame Structure │  Support Status
──────────────┼──────────────────┼────────────────
3.4 - 3.10    │  PyFrameObject   │       ✅
3.11          │  _PyCFrame       │       ✅
3.12          │  _PyCFrame       │       ✅
3.13+         │  current_frame   │       ✅
```

This is achieved through version-specific bindings to Python's internal structures.

## Performance Characteristics

| Metric | Target | Measured |
|--------|--------|----------|
| CPU overhead | < 5% | ~2-3% |
| Memory overhead | < 50MB | ~30MB |
| Query latency | < 10ms | ~5ms |
| Injection time | < 100ms | ~50ms |

## Detailed Comparison with Alternatives

### vs py-spy

| Dimension | Probing | py-spy |
|-----------|---------|--------|
| Core function | Full analysis platform | Sampling profiler |
| Data querying | SQL | Fixed format |
| Code execution | REPL support | Not supported |
| Distributed | Native support | Not supported |
| Use case | AI training debugging | General Python |

### vs torch.profiler

| Dimension | Probing | torch.profiler |
|-----------|---------|----------------|
| Code intrusion | None | Required |
| Runtime attach | Supported | Not supported |
| Query flexibility | SQL | Fixed API |
| Non-PyTorch support | Yes | No |

### vs Perfetto

| Dimension | Probing | Perfetto |
|-----------|---------|----------|
| Focus | AI applications | System tracing |
| Deployment complexity | Low | High |
| Python integration | Native | Limited |
| Learning curve | Low | High |

## Summary

Probing's unique value lies in integrating three powerful capabilities:

1. **Dynamic Injection** - No code changes, runtime attachment
2. **SQL Queries** - Flexible data analysis capabilities
3. **Remote REPL** - Real-time interactive debugging

This combination makes Probing particularly suitable for:

- 🔬 AI researchers debugging training issues
- 🛠️ Framework developers analyzing performance bottlenecks
- 🏭 MLOps engineers monitoring production environments

[Get Started with Probing →](quickstart.md)

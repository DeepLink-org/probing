# Quick Start

Get immediate value from Probing with this streamlined workflow.

## Your First 5 Minutes

### Step 1: Set Your Target Process

All Probing commands need a target endpoint. Set `$ENDPOINT` to either a local process ID or remote address:

```bash
# Local process - find and set your Python process ID
export ENDPOINT=$(pgrep -f "python.*your_script")

# Or for remote processes
export ENDPOINT=remote-host:8080
```

!!! tip "Finding Processes"
    Use `ps aux | grep python` or `pgrep -f "python.*train"` to locate your target.

### Step 2: Connect and Explore

```bash
# Connect to your process (Linux only)
probing $ENDPOINT inject

# Get basic process info
probing $ENDPOINT eval "import os, psutil; proc = psutil.Process(); print(f'PID: {os.getpid()}, Memory: {proc.memory_info().rss/1024**2:.1f}MB')"
```

### Step 3: Try All Three Core Capabilities

#### 📊 Query structured data

```bash
probing $ENDPOINT query "SELECT name, value FROM information_schema.df_settings LIMIT 5"
```

#### 🎯 Execute live code

```bash
probing $ENDPOINT eval "import torch; print(f'CUDA: {torch.cuda.is_available()}')"
```

#### 🔍 Capture execution context

```bash
probing $ENDPOINT backtrace

probing $ENDPOINT query "SELECT func, file, lineno FROM python.backtrace ORDER BY depth LIMIT 5"
```

## Three Core Capabilities

Probing provides three powerful capabilities that work together:

### 🎯 eval: Execute Code in Live Processes

Run arbitrary Python code directly inside your target process:

```bash
# Check training threads
probing $ENDPOINT eval "import threading; [print(f'{t.name}: {t.is_alive()}') for t in threading.enumerate()]"

# Check GPU memory usage
probing $ENDPOINT eval "import torch; print(f'GPU: {torch.cuda.memory_allocated()/1024**3:.1f}GB allocated')"
```

### 📊 query: Analyze Data with SQL

Query structured performance data using familiar SQL syntax:

```bash
probing $ENDPOINT query "
SELECT
    step,
    module,
    SUM(allocated) as total_memory_mb,
    COUNT(*) as operation_count
FROM python.torch_trace
WHERE step > 100
GROUP BY step, module
ORDER BY total_memory_mb DESC
LIMIT 10"
```

### 🔍 backtrace: Debug with Stack Context

Capture detailed call stacks with Python variable values:

```bash
# Capture current call stack
probing $ENDPOINT backtrace

# Query the stack trace
probing $ENDPOINT query "SELECT func, file, lineno FROM python.backtrace ORDER BY depth LIMIT 3"
```

## Real-World Debugging Scenarios

### Scenario 1: Training Process Hanging

**Problem**: PyTorch training suddenly stops progressing.

```bash
# 1. See what main thread is doing
probing $ENDPOINT backtrace

# 2. Check thread states
probing $ENDPOINT eval "import threading; [(t.name, t.is_alive()) for t in threading.enumerate()]"

# 3. Analyze stack context
probing $ENDPOINT query "SELECT func, file, lineno FROM python.backtrace ORDER BY depth LIMIT 10"
```

### Scenario 2: Memory Leak Investigation

**Problem**: Memory usage keeps growing during training.

```bash
# Force cleanup and get current state
probing $ENDPOINT eval "import gc, torch; gc.collect(); torch.cuda.empty_cache()"

# Analyze allocation trends
probing $ENDPOINT query "SELECT step, AVG(allocated) as avg_memory FROM python.torch_trace GROUP BY step ORDER BY step"
```

### Scenario 3: Performance Bottleneck Analysis

**Problem**: Need to identify which model components are slowest.

```bash
# Find most expensive operations
probing $ENDPOINT query "
SELECT module, stage, AVG(duration) as avg_duration
FROM python.torch_trace
GROUP BY module, stage
ORDER BY avg_duration DESC
LIMIT 10"
```

## Next Steps

- [SQL Analytics](guide/sql-analytics.md) - Advanced query techniques
- [Memory Analysis](guide/memory-analysis.md) - Deep dive into memory debugging
- [Debugging Guide](guide/debugging.md) - Expert debugging patterns

# Quick Start

This guide walks you through using Probing to inspect a running training process.
It assumes you've already [installed](installation.md) the package. By the end,
you'll be able to attach to a process, capture backtraces, and run SQL queries
against live performance data.

## Finding and attaching to a process

First, locate the Python process you want to inspect. In your terminal:

```bash
# Find your training process
pgrep -f "python.*train"
# → 27891

# Or with more context
ps aux | grep python | grep -v grep
```

Set the endpoint environment variable so later commands are clean:

```bash
export ENDPOINT=27891
```

On Linux, you can attach to a running process directly:

```bash
probing $ENDPOINT inject
```

On macOS or Windows, injection isn't available — you need to start the process with
probing enabled (`PROBING=1 python train.py`) instead. The `inject` command is the
only part of probing that's Linux-only; everything else (query, eval, backtrace,
skills) works on all platforms once the server is running.

## Your first diagnostic commands

With the server running in the target, grab a backtrace and check what's happening
on the main thread:

```bash
probing $ENDPOINT backtrace
```

The backtrace populates `python.backtrace` — a point-in-time view of the current
stack, mixing Python and native frames. Query it with SQL:

```bash
probing $ENDPOINT query "
  SELECT func, file, lineno, depth, frame_type
  FROM python.backtrace
  ORDER BY depth LIMIT 10
"
```

You'll see function names, source file paths, and whether each frame is Python or
native code. Depth 0 is the innermost frame (what's executing right now).

If you need to inspect live state beyond the stack, use `eval` to run arbitrary
Python in the target:

```bash
probing $ENDPOINT eval "import torch; print(f'CUDA available: {torch.cuda.is_available()}')"
probing $ENDPOINT eval "
  import gc, torch
  gc.collect()
  alloc = torch.cuda.memory_allocated() / 1024**2
  reserved = torch.cuda.memory_reserved() / 1024**2
  print(f'GPU: {alloc:.0f}MB allocated, {reserved:.0f}MB reserved')
"
```

## Three common workflows

What follows are real scenarios — the kind of things you'll actually use Probing for
in practice.

### Training is hanging

The most common use case: training suddenly stops making progress. The process is
alive but nothing is happening.

Start with a backtrace to see what the main thread is doing:

```bash
probing $ENDPOINT backtrace
probing $ENDPOINT query "SELECT func, file, lineno FROM python.backtrace ORDER BY depth LIMIT 5"
```

The innermost frame (depth 0) tells you exactly where execution is stuck. If the
stack shows `ncclAllReduce` or a `torch.distributed` call, you're looking at a
communication hang. If it shows Python code in your model, the computation itself
is the bottleneck.

Next, check thread states — a collective might be hanging while other threads are
fine:

```bash
probing $ENDPOINT eval "
  import threading
  for t in threading.enumerate():
      print(f'{t.name}: alive={t.is_alive()}, daemon={t.daemon}')
"
```

### GPU memory is growing

Memory creeping up step after step — a leak or accumulation pattern. Query the
torch_trace table to see per-step allocation trends:

```bash
probing $ENDPOINT query "
  SELECT local_step, AVG(allocated) as avg_mb, MAX(allocated_delta) as max_delta_mb
  FROM python.torch_trace
  GROUP BY local_step
  ORDER BY local_step
"
```

Look for `allocated_delta` values that don't return to zero between steps —
that indicates memory not being freed between iterations. Pair with `eval` to
force a GC and check current state:

```bash
probing $ENDPOINT eval "import gc, torch; gc.collect(); torch.cuda.empty_cache()"
```

If the GC + cache clear brings memory back down, the issue is Python-side
reference cycles. If not, you're looking at a CUDA-side leak or growing
workspace.

### Finding slow modules and operations

To identify which parts of your model are the bottleneck:

```bash
probing $ENDPOINT query "
  SELECT module, stage, AVG(duration) as avg_duration, COUNT(*) as calls
  FROM python.torch_trace
  WHERE stage IN ('post forward', 'post step')
  GROUP BY module, stage
  ORDER BY avg_duration DESC
  LIMIT 10
"
```

Filtering to `post forward` and `post step` stages gives you the execution times
(the `pre` rows carry timing anchors; the `post` rows carry actual durations).
The results tell you exactly which module and which pass direction accounts for
the most time.

If you're running distributed, add the federated query prefix to compare across
ranks:

```bash
probing -t <master> query "
  SELECT _rank, module, stage, AVG(duration) as avg_duration
  FROM global.python.torch_trace
  WHERE stage IN ('post forward', 'post step')
  GROUP BY _rank, module, stage
  ORDER BY avg_duration DESC
  LIMIT 20
"
```

`_rank` is a federation tag added at query time — see [Core Concepts](guide/concepts.md)
for how this works.

## What's next

These three commands — `backtrace`, `eval`, `query` — cover the majority of
day-to-day diagnostic work. Each is documented in detail in the [API
Reference](api-reference.md).

For deeper analysis patterns (JOINs across tables, time-series bucketing,
statistical aggregation), read [SQL Analytics](guide/sql-analytics.md). For
multi-node debugging, start with [Distributed](design/distributed.md).

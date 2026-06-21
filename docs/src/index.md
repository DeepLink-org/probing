---
template: home.html
title: Probing - Dynamic Performance Profiler for Distributed AI
description: Attach to running Python training processes, query performance data with SQL, and diagnose distributed issues across cluster nodes.
hide: toc
---

# Probing

Probing lets you inspect and debug distributed AI training jobs — without modifying your code.
Attach to a running Python process, query performance data through standard SQL, and run
diagnostic workflows to find slow ranks, NCCL bottlenecks, or memory leaks.

## In 30 seconds

```bash
pip install probing

# Start training with probing enabled
PROBING=1 python train.py &

# Attach and inspect
probing -t $(pgrep -f train.py) backtrace
probing -t $(pgrep -f train.py) query "
  SELECT module, stage, AVG(duration) as sec
  FROM python.torch_trace
  GROUP BY module, stage
  ORDER BY sec DESC LIMIT 5
"
```

The first command starts a training job with probing activated. The second captures
every Python and native frame on the main thread. The third finds the five slowest
module-stage pairs with a single SQL query — no logging, no instrumentation, no restart.

## What you can do with it

**Debug a hanging or slow training job.**
Attach to the stuck process, grab a backtrace, check GPU memory per step, and identify
the exact module or collective call that's blocking. No need to reproduce the issue.

**Profile collective communication at scale.**
The NCCL profiler plugin decomposes proxy-op wait time into send/recv latencies so you
can tell who's waiting on whom — essential for debugging all-reduce tail latency.

**Write custom performance tables.**
Define a dataclass with `@table("my_metrics")`, append rows from your training loop, and
query them alongside built-in tables. Your data lives in `python.my_metrics`, same
namespace, same SQL interface.

**Query across cluster nodes.**
Prefix any table with `global.` to fan out to registered peers. Each row carries `_rank`,
`_role`, `_host` tags so results are directly comparable across the cluster.

## How it works

Probing ships as a Python package with a compiled Rust core (`probing._core`). When you
run `PROBING=1 python train.py`, a `.pth` hook starts an in-process HTTP server and
registers data sources for the SQL engine. Extensions — CPU sampling, GPU memory, NCCL
proxy ops, Python stack tracing — push rows into append-only columnar tables backed by
mmap ring buffers. The CLI talks to the embedded server over a Unix socket (local) or TCP
(remote).

You don't need to know any of that to use it. `pip install probing` and you're done.

## Start here

**I want to debug a training issue.**
Read [Quick Start](quickstart.md), then try `probing backtrace` and `probing query`.

**I want to understand the architecture.**
Read [Core Concepts](guide/concepts.md) and [Modularity & Boundaries](design/modularity.md).

**I'm setting up a multi-node cluster.**
Read [Distributed](design/distributed.md) and the [SQL Tables](reference/sql-tables.md) reference.

**I want to write a custom diagnostic skill.**
Read [Extensibility](design/extensibility.md) and browse `skills/` for examples.

**I want to contribute.**
Read [Contributing](contributing.md), `make develop`, and pick an issue.

## Documentation map

| Doc | Covers |
|-----|--------|
| [Installation](installation.md) | `pip install`, `PROBING=1`, platform support |
| [Quick Start](quickstart.md) | First 5 minutes, real-world debugging scenarios |
| [Core Concepts](guide/concepts.md) | How Probing works — mental model, data flow, step coordinates, federation |
| [SQL Tables](reference/sql-tables.md) | Column reference for every built-in table |
| [API Reference](api-reference.md) | CLI commands and Python API |
| [Environment Variables](reference/env-vars.md) | Complete `PROBING_*` variable reference (30+ entries) |
| [Skill Format](reference/skill-format.md) | `steps.yaml` and `SKILL.md` specification |
| [SQL Analytics](guide/sql-analytics.md) | Query patterns, JOIN examples, time-series |
| [Diagnostic Skills](guide/skills.md) | Running and writing diagnostic workflows |
| [Extensibility](design/extensibility.md) | Data table plugins, diagnostic skills, NCCL profiler |
| [Distributed](design/distributed.md) | Multi-node federation, torchrun integration |
| [NCCL Profiler](design/nccl-profiler.md) | NCCL plugin, proxy-op wait decomposition |
| [Contributing](contributing.md) | Dev setup, pull request workflow |

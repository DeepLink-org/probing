---
template: home.html
title: Probing - Dynamic Performance Profiler for Distributed AI
description: In-process profiler with SQL query interface for Python training workloads.
hide: toc
---

# Probing

In-process profiler for Python training jobs. A Rust core (`probing._core`) embeds an HTTP
server and a DataFusion SQL engine; collectors append rows to mmap memtables; the CLI and
Web UI send queries over a Unix socket or TCP.

## Components

| Component | Location | Function |
|-----------|----------|----------|
| **Probe** | Target Python process | HTTP server, engine, extension registration |
| **Engine** | `probing/core` | DataFusion catalog, federation rewrite, `async_query` |
| **Memtable** | `probing/memtable` | MEMT ring buffers, optional MEMC cold segments |
| **Collectors** | `probing/extensions/*`, `python/probing/` | CPU, GPU, NCCL, torch hooks → SQL tables |
| **CLI / Web** | `probing/cli`, `web/` | HTTP client; no direct engine link at runtime |

Activation: `PROBING=1` at process start (`.pth` hook), or `probing -t <pid> inject` on Linux.

## Minimal example

```bash
pip install probing
PROBING=1 python train.py &

probing -t $(pgrep -f train.py) query "
  SELECT module, stage, avg(duration) AS sec
  FROM python.torch_trace
  GROUP BY module, stage
  ORDER BY sec DESC LIMIT 5
"
```

`backtrace` writes `python.backtrace`; `query` reads registered tables under the `probe`
catalog; `global.*` fans out when cluster membership is configured (see [Distributed overview](design/distributed.md)).

## Documentation

| Section | Content |
|---------|---------|
| [Installation](installation.md) | PyPI, wheel, platforms, `PROBING` modes |
| [Quick Start](quickstart.md) | Attach, inject, first queries |
| [Core model](guide/concepts.md) | In-process vs attach, catalogs, step coordinates |
| [User Guide](guide/index.md) | SQL, skills, debugging commands |
| [Architecture](design/index.md) | Layers, storage, federation, collectors |
| [Reference](reference/index.md) | SQL tables, CLI/API, env vars |

Contributors: [Contributing](contributing.md) · Doc style: [writing.md](writing.md)

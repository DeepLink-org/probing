---
template: home.html
title: Probing - Dynamic Performance Profiler for Distributed AI
description: Zero-intrusion profiler for distributed AI — SQL tables, live introspection, cluster federation, and diagnostic skills.
hide: toc
---

# Probing

**Probing** profiles distributed AI training: continuous SQL tables, live attach, federated
`global.*` queries, and bundled diagnostic skills.

## Capabilities

- **Continuous profiling** — `torch_trace`, `comm_collective`, NCCL proxy, custom `@table`
- **Live introspection** — `eval`, `backtrace`, REPL against running processes
- **SQL analytics** — single-node and `global.*` federation with `_rank` / `_role` tags
- **Diagnostic skills** — `health_overview`, `slow_rank`, `nccl_culprit_victim`, …
- **Cluster** — `cluster nodes`, `cluster query`, Web UI agent
- **Zero intrusion** — `PROBING=1` at startup or Linux `inject`

## Quick Start

```bash
pip install probing

# Recommended for training
PROBING=1 PROBING_TORCH_PROFILING=on python train.py

# Or attach on Linux
probing -t <pid> inject
probing -t <pid> query "SELECT * FROM python.torch_trace LIMIT 10"
probing -t <pid> skill run health_overview
```

## Documentation map

- [Installation](installation.md) · [Quick Start](quickstart.md) · [Core Concepts](guide/concepts.md)
- [SQL Tables](reference/sql-tables.md) · [API Reference](api-reference.md) · [Contributing](contributing.md)

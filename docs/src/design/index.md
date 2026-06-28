# Architecture Overview

Contributor-facing design docs. Operators: **[User Guide](../guide/index.md)**.
Contracts: **[Reference](../reference/index.md)**.

Vocabulary: **[Core model](../guide/concepts.md)**.

## Reading order

1. **[Modularity & boundaries](modularity.md)** — four-layer model, crate map, dependency rules (start here)
2. **[Data Layer](data-layer.md)** — MEMT/MEMC, mmap, SQL integration
3. **[Distributed → Overview](distributed.md)** — multi-node mental model, then nested pages below

## Platform core

| Document | Description |
|----------|-------------|
| [Modularity & boundaries](modularity.md) | L1–L4 layers, public contracts, ownership |
| [Data Layer](data-layer.md) | Hot/cold columnar store and SQL integration |
| [Extensibility](extensibility.md) | `@table` plugins, skills, NCCL profiler hook-in |
| [CLI command tree](cli.md) | Command grouping, target rules, migration (draft) |

## Collectors & profiling

| Document | Description |
|----------|-------------|
| [Profiling](profiling.md) | Torch hooks, sampling, table write path |
| [NCCL Profiler](nccl-profiler.md) | Plugin ABI, proxy-op wait decomposition |
| [Debugging Engine](debugging.md) | eval / backtrace / REPL implementation |
| [Training Phases](training-phase.zh.md) | Phase transitions and span model *(中文)* |

## Distributed

| Document | Description |
|----------|-------------|
| [Overview](distributed.md) | Multi-node topology, control plane, federation intro |
| [Torchrun cluster heartbeat](torchrun-cluster.md) | Hierarchical registration, backoff, env presets |
| [Federated query engine](federation.md) | Cross-rank SQL paths A/B/C, tags, regression queries |
| [Hierarchical fan-out](hierarchical-fanout.md) | Coordinator → local0 → leaf query aggregation |
| [Cluster with Pulsing](cluster-pulsing.md) | Optional Pulsing-based membership |

## Legacy

| Document | Description |
|----------|-------------|
| [System Architecture (legacy)](architecture.md) | Two-layer overview — superseded by [Modularity](modularity.md); kept for historical diagrams |

User-facing workflows: **[User Guide](../guide/index.md)** · Reference: **[SQL Tables](../reference/sql-tables.md)** · **[CLI & Python API](../api-reference.md)**

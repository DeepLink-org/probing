# Architecture Overview

Contributor-facing design docs. Operators: **[User Guide](../guide/index.md)**.
Contracts: **[Reference](../reference/index.md)**.

Vocabulary: **[Core model](../guide/concepts.md)**.

## Reading order

1. **[Modularity & boundaries](modularity.md)** — ownership and dependency direction.
2. **[Activation & runtime control](activation-injection.md)** → **[Data Layer](data-layer.md)** —
   how Probing enters a process and retains evidence.
3. **[Profiling and tracing](profiling.md)** — TorchProbe, spans, training phases, and stacks.
4. **[Distributed membership](distributed.md)** → **[Federation](federation.md)** — membership,
   hierarchical fan-out, and cross-rank SQL.
5. **[Distributed Profiler query and visualization](distributed-profiler.md)** — 10K-rank
   timeline semantics, hierarchical execution, and cross-rank drill-down.

“Current” describes implemented behavior. “Draft/target” pages are not complete product contracts.

## Foundations

| Document | Status | Description |
|----------|--------|-------------|
| [Modularity & boundaries](modularity.md) | Current | Layers, public contracts, ownership |
| [Activation & runtime control](activation-injection.md) | Current | `.pth`, ptrace trampoline, and service readiness |
| [Data Layer](data-layer.md) | Current | MEMT/MEMC hot/cold storage and SQL integration |
| [Extensibility](extensibility.md) | Current | Table, collector, skill, and service contracts |

## Collectors & profiling

| Document | Status | Description |
|----------|--------|-------------|
| [Profiling and tracing](profiling.md) | Current | TorchProbe, spans/phases, Python/native stacks, and system collection |
| [NCCL Profiler](nccl-profiler.md) | Current | Plugin ABI and wait decomposition |
| [Overhead](overhead.md) | Current | Shadow-step formulas, change invariants, and offline benchmarks |

## Distributed query & analysis

| Document | Status | Description |
|----------|--------|-------------|
| [Distributed membership](distributed.md) | Current | Torchrun registration, heartbeat, TTL, and member metadata |
| [Federated query engine](federation.md) | Current | `global.*`, plan selection, hierarchical fan-out, tags, and partial results |
| [Distributed Profiler](distributed-profiler.md) | Target | 10K-rank timelines, hierarchical query, multi-resolution views, and flamegraphs |

User-facing workflows: **[User Guide](../guide/index.md)** · Reference: **[SQL Tables](../reference/sql-tables.md)** · **[CLI & Python API](../api-reference.md)**

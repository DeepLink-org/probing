# User Guide

Command and SQL usage for operators. Contracts (columns, HTTP, env vars) live in
**[Reference](../reference/index.md)**; internals in **[Architecture](../design/index.md)**.

Terminology: **[Core model](concepts.md)**.

## Interfaces

| Interface | Input | Output / side effect |
|-----------|-------|-------------------|
| `probing query` | SQL on `probe.*` / `global.*` | `DataFrame` JSON |
| `probing eval` | Python source | stdout / exception in target interpreter |
| `probing backtrace` | — | Rows in `python.backtrace` |
| `probing skill run` | skill id + params | Steps from `skills/*/steps.yaml` |
| `probing cluster nodes` | — | `cluster.nodes` registry view |
| `probing cluster query` | SQL + `cluster=true` | Fan-out per [Federation](../design/federation.md) |

Built-in tables: `python.torch_trace`, `python.comm_collective`, `gpu.utilization`, `nccl.proxy_ops`, … — see **[SQL Tables](../reference/sql-tables.md)**.

## Page order

1. [Next Web UI](web-ui.md) — evidence scope, investigation path, page semantics
2. [SQL Analytics](sql-analytics.md) — `global.*`, federation tags, JOIN rules
3. [Diagnostic Skills](skills.md) — `steps.yaml` runner
4. [Memory Analysis](memory-analysis.md) — `python.torch_trace`, GPU tables
5. [Live Debugging](debugging.md) — backtrace, eval, REPL
6. [Troubleshooting](troubleshooting.md) — HTTP, inject, empty tables

Getting started path: Installation → Quick Start → Core model (nav **Getting Started**).

## Architecture pointers

- [Modularity & boundaries](../design/modularity.md) — L1–L4 layers
- [Distributed overview](../design/distributed.md) — membership, fan-out
- [Extensibility](../design/extensibility.md) — `@table`, skills

CLI flags: **[CLI & Python API](../api-reference.md)**.

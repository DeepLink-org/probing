# User Guide

How-to guides for analyzing and debugging AI training with Probing.

## What Probing does

| Layer | What you get | How |
|-------|--------------|-----|
| **Continuous profiling** | `python.torch_trace`, `python.comm_collective`, spans, plugins | Hooks append rows as training runs |
| **Live introspection** | Inspect live objects, capture stacks | CLI `eval`, `backtrace` (or in-process API) |
| **SQL analytics** | Ad-hoc and federated queries | `query`, `global.*`, `cluster query` |
| **Diagnostic skills** | Curated multi-step investigations | `probing skill run <id>` |

Terminology anchor: **[Core Concepts](concepts.md)**. Table columns: **[SQL Tables](../reference/sql-tables.md)**.

## Reading order

New users — **Getting Started** in the nav: Installation → Quick Start → Core Concepts.

Then this guide:

1. **[SQL Analytics](sql-analytics.md)** — queries, `global.*`, `_role`
2. **[Diagnostic Skills](skills.md)** — `health_overview`, `slow_rank`, …
3. **[Memory Analysis](memory-analysis.md)** — leaks and GPU pressure
4. **[Debugging](debugging.md)** — backtrace / eval workflows
5. **[Troubleshooting](troubleshooting.md)** — common failures

## Primary CLI commands

| Command | Role |
|---------|------|
| `query` | Read profiling tables |
| `eval` | Run Python in the target process |
| `backtrace` | Capture stack → `python.backtrace` |

Full CLI reference: **[API Reference](../api-reference.md)**.

## Design docs

- **[Architecture](../design/architecture.md)** — probe, engine, extensions
- **[Distributed](../design/distributed.md)** — cluster, federation
- **[Extensibility](../design/extensibility.md)** — `@table` plugins

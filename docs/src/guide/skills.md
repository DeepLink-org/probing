# Diagnostic Skills

**Skills** are versioned, multi-step SQL playbooks for common training investigations.
They ship in the wheel (`python/probing/bundled_skills/` via `probing.skills` entry point),
execute via the shared Rust runner (`probing-skills`: CLI, Web Agent, MCP), and are
discovered at runtime from bundled trees, repo checkout, and installed `probing-<vendor>`
packages.

## When to use skills vs raw SQL

| Approach | Best for |
|----------|----------|
| **`probing skill run <id>`** | Known scenario (hang, slow rank, memory leak) — curated steps + thresholds |
| **`probing query "…"`** | Ad-hoc exploration, custom dashboards |
| **`cluster query` + `global.*`** | Cross-rank comparison with federation tags |

Skills read the same tables documented in **[SQL Tables](../reference/sql-tables.md)**.
Use `step_snapshot()` coordinates and `_role` / `_rank` tags — not framework-specific
`trainer.current_step` in skill parameters.

## Quick start

```bash
export ENDPOINT=rank0:8080

# List bundled skills
probing $ENDPOINT skill list

# Entry triage (CPU/GPU/tables/recent activity)
probing $ENDPOINT skill run health_overview

# Distributed scenarios — fan out global.* when cluster peers exist
probing $ENDPOINT skill run slow_rank --global
probing $ENDPOINT skill run nccl_culprit_victim --global
```

Override parameters:

```bash
probing $ENDPOINT skill run module_bottleneck -p window_steps=50 -p top_n=15
```

## Bundled skills (0.2.x)

| ID | Category | Purpose |
|----|----------|---------|
| `health_overview` | Triage | First look: utilization + table freshness |
| `training_hang` | Reliability | Stalls, idle threads, missing steps |
| `slow_rank` | Distributed | Straggler ranks via `global.*` |
| `nccl_culprit_victim` | Distributed | Collective wait imbalance |
| `comm_bottleneck` | Distributed | Communication vs compute ratio |
| `module_bottleneck` | Performance | Hot modules in `torch_trace` |
| `gpu_pressure` | Memory | VRAM pressure patterns |
| `memory_leak` | Memory | Growing allocations over steps |

Run `probing skill list` for the authoritative list on your install.

## Install into coding agents

Copy skills into agent skill directories (Cursor, Claude Code, Codex):

```bash
probing skill install --user
probing skill update --user
```

Authoring tree lives in repo `skills/`; validate before wheel:

```bash
python -m probing.skills validate
make wheel   # bundles skills/ into the wheel automatically
```

## Authoring

Each skill is a folder:

```
skills/<id>/
  SKILL.md      # intent, when to use, human summary
  steps.yaml    # ordered SQL steps with params
```

Agent routing metadata (`intents.yaml`, `pages.yaml`) lives under `skills/semantic/`.
Table/column descriptions are code-first in collectors and `@table`; `probing/core/resources/tables.yaml`
is an L1 agent overlay (synonyms, notes, `global_name`) merged at engine startup into
`probe.probing.table_docs` / `column_docs`. See **[Contributing](../contributing.md#skills-agents)**.

## Related

- **[SQL Analytics](sql-analytics.md)** — `global.*`, `_role` GROUP BY
- **[Core Concepts](concepts.md)** — federation tags, step coordinates
- **[API Reference](../api-reference.md)** — `skill` / `cluster` CLI

# Agent instructions — probing

This repository uses the [Agent Skills](https://agentskills.io) layout for training diagnostics.

## Module boundaries

Before adding code, read **`docs/src/design/modularity.md`** (中文: `modularity.zh.md`).

| Layer | Where | Your change belongs if… |
|-------|--------|-------------------------|
| L1 Platform | `probing/core`, `memtable`, `proto` | SQL engine, federation, storage format |
| L2 Collectors | `probing/extensions/*`, `python/probing/profiling` | New metrics / tables |
| L3 Control | `probing/server`, `probing/cli` | HTTP, inject, fan-out |
| L4 Experience | `skills/`, `web/`, Python hooks | Diagnostics UX, skills, UI |

**Contracts:** `ProbeDataSource` (tables), `ProbeExtension` (config/HTTP), `@table` (Python data), `skills/*/steps.yaml` (workflows). Do not add cross-collector calls — use SQL JOINs.

## Skills

All diagnostic skills live under **`skills/`**. Each subdirectory contains:

- **`SKILL.md`** — when to use the skill and how to interpret results (read this for routing)
- **`steps.yaml`** — executable probe steps (used by `probing skill run` and the Web Investigate agent)

Browse the catalog: `skills/catalog.yaml`

## Install skills into your agent

So Cursor / Claude Code / Codex can discover and invoke skills:

```bash
./skills/install.sh
```

This copies `skills/<id>/` into:

- `.cursor/skills/` (Cursor)
- `.claude/skills/` (Claude Code)
- `.agents/skills/` (Codex)

Use `probing skill install --user` for global install under `~/`.

## Run diagnostics

Requires a probed training process (`PROBING=1` or `probing -t <pid> inject`):

```bash
probing skill list
probing -t <pid> skill run health_overview
probing -t <pid> skill run slow_rank --global
probing -t <pid> skill run nccl_culprit_victim
```

From Python (e.g. in agent-generated scripts):

```python
from probing.skills.tools import list_skills, run_skill
run_skill("health_overview", target="<pid>")
```

## Built-in skills (summary)

| id | use when |
|----|----------|
| `health_overview` | first look / triage |
| `training_hang` | stall or hang |
| `slow_rank` | straggler rank |
| `comm_bottleneck` | NCCL / collective slow |
| `nccl_culprit_victim` | NCCL culprit/victim analysis |
| `memory_leak` | GPU memory growth |
| `module_bottleneck` | slow PyTorch modules |
| `gpu_pressure` | GPU util / headroom |

Details in each `skills/<id>/SKILL.md`.

## Extending

Add table plugins under `python/probing/ext/` (data). Add diagnostic skills under `skills/` (how to investigate). NCCL profiler plugin: `docs/src/design/nccl-profiler.md`. See `docs/src/design/extensibility.md`.

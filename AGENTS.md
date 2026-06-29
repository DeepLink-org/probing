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

## Error handling & runtime

Keep the propagation chain clean — don't reintroduce scattered `map_err`/`inspect_err`.

- **One error type per crate.** `probing/core` funnels everything into `EngineError` (`probing/core/src/core/error.rs`); app layers (`server`, `cli`) use `anyhow`. Prefer `?` with `#[from]`/`#[source]`; never add `From<String>`, and don't `map_err` an error into a flat string (it drops the cause chain). Attach context with `anyhow::Context` (`.context` / `.with_context`).
- **One boundary conversion.** `EngineError → DataFusionError` lives in a single `From` impl; DataFusion trait impls just use `?` instead of hand-rolling `DataFusionError::Execution(format!(...))`.
- **`block_on` never fabricates data.** `probing_core::runtime::block_on` returns `Result<T, RuntimeError>` — on a degraded async bridge it returns `Err`, not an empty/`default` value. Surface it; for a diagnostics tool, a silent "no data" is worse than a clear failure.
- **No `unwrap`/`expect`/`panic!` on production paths.** Propagate with `?`. Exempt: tests and vendored py-spy bindings under `probing/extensions/python/src/features/spy/python_bindings/`.
- **Never pollute the host process's stdout.** Use `log` (Rust) / `logging` (Python), not `print!`/`println!`. Expected contention/busy states (e.g. a concurrent stack-trace request) belong at `debug`, not `error`.

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
probing -t <pid> skill run job_health --global
probing -t <pid> skill run slow_rank --global
probing -t <pid> skill run persistent_straggler --global
probing -t <pid> skill run nccl_culprit_victim
```

### MCP (coding agents)

When the probing server is running (e.g. after `PROBING=1`), connect your agent to **`http://<host>:<port>/mcp`** (Streamable HTTP).

**Read tools:** `query`, `describe_tables`, `list_skills`, `plan_skill`, `run_skill`, `list_cluster_nodes`, `cluster_query`

**Write tools (opt-in):** `set_config`, `eval_python` — require `PROBING_MCP_ALLOW_WRITE=1`

**Resources:** `probing://schema/catalog`, `probing://schema/{schema}/{table}`

See `probing/server/API.md`.

From Python (e.g. in agent-generated scripts):

```python
from probing.skills.tools import list_skills, run_skill
run_skill("health_overview", target="<pid>")
```

## Built-in skills (summary)

| id | use when |
|----|----------|
| `health_overview` | first look / triage |
| `job_health` | job-level slowdown, step lag, cluster alive |
| `training_hang` | stall or hang |
| `slow_rank` | straggler rank (current window) |
| `persistent_straggler` | chronic straggler (worst_fraction) |
| `comm_bottleneck` | NCCL / collective slow |
| `nccl_culprit_victim` | NCCL culprit/victim analysis |
| `memory_leak` | GPU memory growth |
| `module_bottleneck` | slow PyTorch modules |
| `gpu_pressure` | GPU util / headroom |

Details in each `skills/<id>/SKILL.md`.

## Extending

Add table plugins under `python/probing/ext/` (data). Add diagnostic skills under `skills/` (how to investigate). NCCL profiler plugin: `docs/src/design/nccl-profiler.md`. See `docs/src/design/extensibility.md`.

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

## Coding principles

These rules extend `docs/src/design/modularity.md`. Read that doc for layer ownership and dependency direction; follow the rules below on every change.

### Minimal diff (pragmatic)

- **Solve the stated problem only.** No cross-module drive-by refactors, renames, or formatting.
- **In touched files, necessary follow-on edits are OK** — fix what the change breaks in the same module; do not expand scope to unrelated modules.
- **One concern per PR.** A bug fix should not also add a feature; a skill SQL tweak should not also change server routing.
- **Composition stays at the root.** New wiring belongs in `probing/server/src/engine.rs` (or the relevant composition root), not sprinkled across L2 collectors.
- **Prefer under 500 lines diff.** Larger PRs need a short justification in the description.

**Refactoring in the same PR** is allowed only when **under 50 lines and confined to one module/crate**. Larger refactors → separate PR first.

### High cohesion, low diffusion

Each crate / directory owns one concern end-to-end. Default: **≤2 modules/crates** per PR. A third module is OK with justification in the PR description.

**Counted as one logical change (not diffusion):** `probing/server` + `probing/proto` + `tests/regression/spec/api_spec.json` — the standard HTTP contract triple.

| Good (1–2 modules) | Bad (spaghetti) |
|--------------------|-----------------|
| New NCCL column → `probing-nccl-profiler` only | Collector imports server types to fan out |
| Federation tag fix → `probing-core/federation` | Skill YAML calls a one-off HTTP endpoint |
| Skill step SQL → `skills/<id>/steps.yaml` | Server special-cases one skill's query |
| HTTP endpoint → server + proto + api_spec | CLI links `probing-core` at runtime |

**When you touch an area, fix boundary violations there** (forbidden imports, collector cross-calls) if the cost is reasonable — do not leave the region worse, and do not use a feature PR to pay unrelated tech debt elsewhere.

**Diffusion smell checklist** — fix before merge:

1. Import crosses a forbidden edge (see modularity §4)?
2. Same logic duplicated in two collectors? → push to L1 or express as SQL.
3. Feature flag or `if skill == "…"` in server/engine?
4. Change required edits in unrelated layers "for consistency"?

### Contractual boundaries, not noodles

Modules interact through **published contracts** only. Implementation details stay private.

| Contract | Consumers may… | Consumers must NOT… |
|----------|----------------|---------------------|
| `ProbeDataSource` | `SELECT` from table; read schema/docs | Import collector internals |
| `ProbeExtension` | HTTP routes, config keys in `API.md` | Call other extensions directly |
| `@table` / `python.*` | SQL JOIN across schemas | Python collector → Python collector calls |
| `skills/*/steps.yaml` | SQL + documented HTTP only | Import Rust/Python engine code |
| `probing/proto` DTOs | CLI, Web, tests over HTTP | Link `probing-core` from CLI/Web |
| Federation tags (`_host`, `_rank`, …) | Filter/group in SQL | Invent alternate peer columns |

**When a contract is insufficient:** extend it first (new table, extension route, proto field). A tiny one-off may use an internal API **only with** a tracked issue/TODO and a plan to fold into the contract later.

**Noodle anti-patterns** (block merge):

- Callback chains between collectors.
- Type leakage: L3 imports L2 internals; CLI/Web links `probing-core`.
- Stringly coupling: magic table names in server without catalog/docs.
- Side-door `pub fn` for a distant caller instead of extending the contract.

### Dependencies & feature gates

- **L1 (`probing/core`, `memtable`, `proto`):** new Rust crates allowed when justified in the PR.
- **L2/L3/L4:** prefer std library and existing workspace deps; avoid new crates/pip packages unless necessary.
- **New behavior:** gate with **env/config** (`PROBING_*`, documented keys) — avoid compile-time `feature` explosion unless platform-gating (Linux-only, etc.).

### Breaking changes (0.x Beta)

Public contracts (table columns, HTTP/MCP, skill output semantics) **may break** during Beta, but every breaking change must be **listed explicitly** in the PR description (and CHANGELOG when applicable), with regression/contract tests and docs updated.

### Documentation

Update docs for anything **user-visible**: CLI behavior, env vars, table schema, skill interpretation, HTTP/MCP tools. Minimum touch points: relevant `docs/src/` page, `API.md` / `SKILL.md` as applicable.

### Code review priorities

Reviewers should **block on boundary violations** before nitpicking style: wrong layer, contract bypass, missing test tier, module diffusion. Correctness matters equally once boundaries are clean.

### Change checklist (before opening a PR)

1. Which **layer** owns this? (`modularity.md` §6 concern table)
2. How many **modules/crates**? Default ≤2; server+proto+api_spec = one logical change.
3. Crossed boundaries only via a **contract** (or internal API + issue)?
4. Tests in the **right tier** — TDD preferred: failing test first, then implementation.
5. User-visible change → **docs** updated?
6. Breaking contract → **called out** in PR description?

### TorchProbe overhead (do not regress)

Changing overhead **formulas**, **`_close_step_wall` hook order**, or **deferred drain defaults** requires reading **`docs/src/design/overhead-invariants.zh.md`** and updating the tests listed there.

| Invariant | Do not |
|-----------|--------|
| Primary % | Replace median ratios with `mean(probed)/mean(shadow)` |
| Amortized | Use mean amortization; must be `(1−rate)×dispatch + rate×sampled` |
| Hook order | Call `_drain_deferred()` before `_record_step_timing()` |
| Async drain | Remove default `PROBING_TORCH_DEFER_ASYNC=1` without replacement |

```bash
cd web && cargo test overhead
PROBING=0 pytest tests/regression/profiling/test_overhead_invariants.py tests/regression/profiling/test_torch_probe_sampling.py -q
```

## Testing

Full layout: `tests/README.md`. Two tiers: **unit** (fast, isolated) and **regression** (integration, contract, E2E). **Prefer TDD:** write a failing test, then implement.

### Where tests live

| Kind | Rust | Python |
|------|------|--------|
| **Unit** | `#[cfg(test)]` modules **inside** the crate under test (`probing/**/src/**/*.rs`) | `tests/unit/probing/…` — **mirror** `python/probing/` layout |
| **Regression / integration** | `tests/regression/rust/probing/<crate>/…` (workspace crate `probing-rust-regression`) | `tests/regression/<area>/…` (`core/`, `ext/`, `nccl/`, `skills/`, …) |
| **Contract** | Same regression crate + `tests/regression/spec/` (`api_spec.json`) | `tests/regression/spec/test_api_spec.py` |
| **Crate-local integration** | Rare: `probing/<crate>/tests/*.rs` when tightly bound to one crate (e.g. `probing/server/tests/`) | — |

**Not test directories:** `python/probing/testing/` — shared factories/fixtures only.

### What belongs in each tier

**Unit** — required for **new functions, branches, and bug fixes** in logic paths; pure wiring/glue may omit unit tests:

- Pure functions, parsers, federation rewrite, skill YAML loader, env helpers.
- No live engine, no network (unless testing memtable itself).
- Rust: `make test-rust-unit`. Python: `make test-python-unit` (`tests/unit/conftest.py` → `PROBING=0`).

**Regression** — required when changing **public surface**: HTTP/MCP endpoints, federation behavior, skill output semantics, cross-crate integration:

- `POST /query`, cluster fan-out, torchrun heartbeat, NCCL mock tables, training_observability E2E.
- Internal refactors with unchanged behavior may rely on existing regression tests.
- Python: `make test-python-regression` (`PROBING=1`). Rust: `make test-rust-regression`.

**Contract** — any HTTP/MCP change:

- Update `tests/regression/spec/api_spec.json` and run `pytest tests/regression/spec/ -q`.

### Test placement rules

1. **Test the module you changed** — unit tests next to (Rust) or mirrored under (Python) that module.
2. **One integration test per boundary** — do not duplicate the same federation/API check in server and CLI.
3. **No production imports from `tests/`** — tests use public APIs and contracts only.
4. **Skills** — loader/interpret unit tests in `tests/unit/probing/skills/`; E2E in `tests/regression/skills/` or `tests/regression/nccl/`.
5. **Markers** — `unit`, `regression`, `integration`, `slow` (`pyproject.toml`); expensive tests → `slow`, not unit.

### Commands (run what you touched)

```bash
make test-rust-unit          # Rust unit (in-crate)
make test-rust-regression    # Rust integration
make test-python-unit        # Python unit
make test-python-regression  # Python integration
python -m probing.skills validate   # after skills/ edits
pytest tests/regression/spec/ -q    # after API.md / HTTP changes
```

## Error handling & runtime

Keep the propagation chain clean — don't reintroduce scattered `map_err`/`inspect_err`.

- **One error type per crate.** `probing/core` funnels everything into `EngineError` (`probing/core/src/core/error.rs`); app layers (`server`, `cli`) use `anyhow`. Prefer `?` with `#[from]`/`#[source]`; never add `From<String>`, and don't `map_err` an error into a flat string (it drops the cause chain). Attach context with `anyhow::Context` (`.context` / `.with_context`).
- **One boundary conversion.** `EngineError → DataFusionError` lives in a single `From` impl; DataFusion trait impls just use `?` instead of hand-rolling `DataFusionError::Execution(format!(...))`.
- **`block_on` never fabricates data.** `probing_core::runtime::block_on` returns `Result<T, RuntimeError>` — on a degraded async bridge it returns `Err`, not an empty/`default` value. Surface it; for a diagnostics tool, a silent "no data" is worse than a clear failure.
- **No `unwrap`/`expect`/`panic!` on production paths.** Propagate with `?`. Exempt: tests and vendored py-spy bindings under `probing/extensions/python/src/features/stacktrace/spy/python_bindings/`.
- **Never pollute the host process's stdout.** Use `log` (Rust) / `logging` (Python), not `print!`/`println!`. Expected contention/busy states (e.g. a concurrent stack-trace request) belong at `debug`, not `error`.

## Skills

All diagnostic skills live under **`skills/`** (authoring SSOT; wheel copy in
`python/probing/bundled_skills/`). Each subdirectory contains:

- **`SKILL.md`** — when to use the skill and how to interpret results (read this for routing)
- **`steps.yaml`** — executable probe steps (used by `probing skill run` and the Web Investigate agent)

Browse the catalog: `skills/catalog.yaml`

**Architecture:** *Discovery* — Python entry points (`python -m probing.extensions skill-roots`)
or server API (`GET /apis/pythonext/skills/*`). *Execution* — Rust `probing-skills`
(CLI in-process, MCP `run_skill` / `plan_skill`, Web WASM). Python `probing.skills.tools`
is discovery/plan only.

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

From Python (discovery / catalog only — execution is Rust CLI or MCP):

```python
from probing.skills.tools import list_skills, plan_skill_run
plan_skill_run("health_overview")
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
| `watchdog_timeout` | NCCL watchdog / Flight Recorder collective alignment |
| `sre_triage` | on-call first-response runbook |
| `crash_triage` | post-crash log / spill triage |

Details in each `skills/<id>/SKILL.md`.

## Extending

- **In-repo skills:** `skills/` + `python -m probing.skills validate`
- **Vendor pip packages:** `probing-<vendor>` — discovery in `python/probing/extensions/`; template `examples/probing-acme/`
- **Table plugins:** `python/probing/ext/` (`@table` + `python.enabled`)
- **NCCL profiler:** `docs/src/design/nccl-profiler.md`

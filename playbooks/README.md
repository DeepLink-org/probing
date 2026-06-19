# Probing Playbooks

Playbooks are structured diagnostic runbooks shared by:

- **CLI** — `probing doctor <playbook-id>` (planned)
- **Web Agent** — LLM selects a playbook, fills parameters, runs steps, explains results
- **Docs** — golden SQL examples with stable IDs

## Layout

```
playbooks/
├── catalog.yaml              # Index of all playbooks
├── semantic/
│   └── tables.yaml           # Schema + synonyms for agent grounding
└── diagnostics/
    ├── health_overview.yaml
    ├── training_hang.yaml
    └── ...
```

## Format (`apiVersion: probing.dev/v1`)

Each playbook is a YAML file with two top-level sections:

| Section | Purpose |
|---------|---------|
| `metadata` | Identity, triggers, tags, human docs |
| `spec` | Parameters, prerequisites, steps, interpretation rules |

### Step types

| `type` | Runs via | Use for |
|--------|----------|---------|
| `sql` | `POST /query` | Read-only analytics (primary) |
| `api` | HTTP GET/POST | Backtrace, flamegraph, overview |
| `config` | `SET probing.*` | Enable sampling before re-query |
| `eval` | `/apis/pythonext/eval` | Last resort; requires confirmation in agent mode |
| `ui` | Web only | Navigate, open panel, set investigation context |

### Parameter templating

Steps use `{param}` placeholders expanded from `spec.parameters` defaults and runtime overrides.

Built-in variables (injected by runner):

| Variable | Meaning |
|----------|---------|
| `{pid}` | Target process id |
| `{global_prefix}` | `global.` when cluster fan-out enabled, else `` |
| `{step_recent}` | `MAX(step)` or `MAX(global_step)` subquery for recent window |

### SQL safety (enforced by runner)

- Only `SELECT`, `WITH`, `SHOW`, `DESCRIBE` allowed in `sql` steps
- Auto-append `LIMIT` if missing and step has no aggregate
- `config` steps that change sampling require explicit `confirm: true` in agent UI

### Interpretation rules

`spec.interpretation.rules` provide **deterministic** summaries before/alongside LLM narration:

```yaml
interpretation:
  rules:
    - id: straggler_detected
      when: "step:rank_latency | column:avg_ms | max/min(ratio) > 1.5"
      severity: warning
      message: "Rank {worst_rank} avg collective latency is {ratio}x the median"
```

Rule expression syntax is intentionally simple in v1; see `python/probing/playbooks/interpret.py`.

## Adding a playbook

1. Create `diagnostics/my_playbook.yaml`
2. Register in `catalog.yaml`
3. Add table/column synonyms to `semantic/tables.yaml` if new data sources are used
4. Run `python -m probing.playbooks validate` (loads + checks SQL placeholders)

## Web UI (Investigation Agent)

The Dioxus web app embeds the same playbooks:

- **Toggle**: `Agent` button or **⌘J / Ctrl+J**
- **LLM (optional)**: ⚙ settings → API base + key + model → saved in **browser localStorage** (`probing_llm_config`). Defaults: DeepSeek (`https://api.deepseek.com/v1`, `deepseek-chat`).
- Without LLM: keyword routing + playbook steps
- With LLM: model picks playbook + parameters, then summarizes SQL evidence
- **Step cards**: each playbook step (SQL / API / navigate) renders as a collapsible card in the chat — reuses `DataFrameView`, `StatCard`, and sidebar-style **Open →** links to jump to profiling/analytics views

Endpoint must be OpenAI-compatible and allow **browser CORS**. The Web UI uses **`async-openai`** (v0.41+) for chat completions.

## Design principles

1. **Playbook before free-form SQL** — LLM picks playbook + fills params, not invent queries from scratch
2. **Evidence chain** — every step produces named output referenced in `summary_template`
3. **Graceful degradation** — `requires` + `on_empty: skip|warn|abort` per step
4. **Same YAML, two consumers** — CLI runs sql/api/config; Web also runs `ui` steps

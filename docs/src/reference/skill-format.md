# Skill Format

Reference specification for Probing diagnostic skills. A skill is a YAML + Markdown
package that defines a diagnostic workflow: SQL queries to run, interpretation rules
to apply, and findings to produce.

Skills live in `skills/<skill_id>/` with two files: `steps.yaml` (the executable
workflow) and `SKILL.md` (agent-facing documentation).

## Directory layout

```
skills/
├── catalog.yaml                  Master index of all skills
├── semantic/
│   ├── tables.yaml               Symlink → probing/core/resources/tables.yaml (agent overlay)
│   ├── intents.yaml              Keyword-to-skill routing
│   └── pages.yaml                Page-to-skill suggestions
└── <skill_id>/
    ├── steps.yaml                Executable diagnostic workflow
    └── SKILL.md                  Agent-facing markdown (frontmatter + body)
```

## catalog.yaml

The master index. Each entry maps a skill ID to its category, priority, and path.

```yaml
apiVersion: probing.dev/v1
kind: Catalog
categories:
  triage:
    label: "Triage"
    description: "Quick health checks and first-response diagnostics"
skills:
  - id: health_overview
    category: triage
    priority: 100
    entry: true
    description: "Quick system health overview — GPU memory, CPU, process status"
    tables:
      - python.torch_trace
    pages:
      - dashboard
    related:
      - slow_rank
    path: health_overview/steps.yaml
```

**Fields:**
- `priority`: Higher values surface the skill earlier in listings and LLM selection.
- `entry: true`: This skill is recommended as an entry point for its category.
- `tables`: Tables the skill queries. Used for prerequisite checking.
- `pages`: Web UI pages where this skill is relevant.
- `related`: Skills commonly used before or after this one.

## steps.yaml

The executable workflow. A single YAML file with schema version, metadata, steps,
interpretation rules, and summary template.

### Top-level structure

```yaml
apiVersion: probing.dev/v1
kind: Skill

metadata:
  id: my_skill
  title: "My Diagnostic Skill"
  title_en: "My Diagnostic Skill"
  category: performance
  tags: [gpu, memory]
  triggers:
    keywords:
      zh: ["GPU", "显存"]
      en: ["GPU", "memory"]
  docs: |
    Detailed description shown in CLI and web agent.
    Can span multiple lines.

spec:
  parameters: [...]
  requires: {...}
  steps: [...]
  variables: {...}
  interpretation:
    rules: [...]
  summary_template: "..."
  next_steps: [...]
```

### Parameters

Declare typed parameters users can override with `--set key=value`.

```yaml
parameters:
  - name: sample_limit
    type: integer
    default: 100
    description: "Maximum rows to return per query"
  - name: use_global
    type: boolean
    default: true
    description: "Use global.* federation when available"
  - name: step_window
    type: integer
    default: 20
    description: "Number of recent steps to analyze"
```

Types: `integer`, `boolean`, `string`. Parameter values are referenced in SQL as
`{param_name}`.

### Requires

Prerequisite check. The skill won't run if requirements aren't met.

```yaml
requires:
  any_tables:
    - python.torch_trace
    - nccl.proxy_ops
```

At least one of the listed tables must exist on the target endpoint.

### Steps

Ordered list of diagnostic operations. Each step has a type:

**`sql`** — Run a SQL query.

```yaml
steps:
  - id: check_gpu_mem
    title: "GPU Memory Per Step"
    type: sql
    sql: |
      SELECT local_step, AVG(allocated) as avg_mb, MAX(max_allocated) as peak_mb
      FROM {var_table}
      WHERE local_step > (SELECT MAX(local_step) FROM {var_table}) - {step_window}
      GROUP BY local_step
      ORDER BY local_step
    on_empty: warn
    empty_message: "No GPU memory data found. Is PROBING_TORCH_PROFILING=on?"
    cluster: false
```

Step fields:
- `id`: Unique within the skill.
- `title`: Displayed in output as `## Title`.
- `type`: `sql` (default), `api`, `ui`, or `config`.
- `sql`: The SQL query. `{param}` and `{var_name}` templates are expanded at runtime.
- `on_empty`: `skip` (default), `warn` (show empty message), or `abort` (stop).
- `empty_message`: Shown when the query returns zero rows (if `on_empty` is not `skip`).
- `cluster`: If `true`, uses federation fan-out (`POST /apis/cluster/query`).
- `when`: Optional condition. `"always"` or `"{use_global}"` (runs only when the
  boolean variable is true).

**`api`** — Call an HTTP API on the probing endpoint.

```yaml
  - id: check_nodes
    title: "Cluster Peers"
    type: api
    path: /apis/nodes
    method: GET
```

**`ui`** — Navigate the web UI to a view (web agent only; skipped in CLI).

```yaml
  - id: show_training
    title: "Training Dashboard"
    type: ui
    view: training
```

**`config`** — Read or suggest a config change (CLI skips; web agent presents to user).

```yaml
  - id: check_sampling
    title: "Sampling Rate Check"
    type: config
    config_key: probing.cpu.sample.interval
```

### Variables

Derived template variables. The `comm_table` and `nccl_proxy_table` variables are
pre-defined and resolve based on `use_global`:

```yaml
variables:
  comm_table: "{use_global ? global.python.comm_collective : python.comm_collective}"
  nccl_proxy_table: "{use_global ? global.nccl.proxy_ops : nccl.proxy_ops}"
```

The system derives these automatically unless overridden. `use_global` is itself a
parameter (default `true`) that auto-detects cluster availability from
`GET /apis/nodes`.

### Interpretation rules

Rules evaluate query results and produce severity-graded findings. They run after
all steps complete. Clauses are joined with `|` (all must match).

```yaml
interpretation:
  rules:
    - id: no_tables
      when: "step:available_tables | rows == 0"
      severity: error
      message: "No probe tables — confirm PROBING=1"
    - id: straggler
      when: "step:rank_latency | column:avg_ms | max/min(ratio) > 1.5"
      severity: warning
      message: "Rank spread on avg_ms exceeds 1.5×"
    - id: chronic
      when: "step:worst_fraction | rows >= 2 | top(row).worst_fraction > 2 * median(worst_fraction)"
      severity: info
      message: "One rank dominates worst_fraction"
    - id: cluster_dead
      when: "step:cluster_nodes | column:status | any_contains('dead')"
      severity: warning
      message: "Dead node in cluster.nodes"
```

**`when` grammar (implemented in `probing-skills` / `interpret.rs`):**

| Fragment | Example |
|----------|---------|
| Step binding | `step:<step_id>` — required for row/column predicates |
| Row count | `rows == 0`, `rows >= 1` |
| Column predicate | `column:<name> \| <tail>` — pairs with the tail on the right |
| Numeric compare | `value == 0`, `value > 10`, `max > 1e6`, `avg > 5` |
| Spread | `max/min(ratio) > 1.5` |
| Ratio of columns | `ratio(num_col/den_col) > 0.3` |
| Text search | `any_contains('dead', 'stale')` — case-insensitive substring |
| Top vs median | `top(row).<col> > N * median(<col>)` — needs `rows >= 2` on the step |

Rule fields:
- `id`: Unique within the skill.
- `when`: Predicate string (see table above). **Not** arbitrary expressions like `slope()` or `avg(col) > 90% * gpu_total` unless implemented in `interpret.rs`.
- `severity`: `error`, `warning`, or `info`.
- `message`: Template; `{step_id.column}` and `{column}` placeholders expand from the first matching row.

### Summary template

A template expanded with step result metadata after the run:

```yaml
summary_template: |
  ## Summary
  - GPU Memory: {check_gpu_mem.row_count} data points across {step_window} steps
  - Slowest collective: {check_collective.top_op} at {check_collective.max_duration_ms:.1f}ms
```

Available fill values: `{step_id.row_count}`, `{step_id.max_<column>}`,
`{step_id.min_<column>}`, `{step_id.top_<column>}`.

### Next steps

Suggestions shown after the skill completes:

```yaml
next_steps:
  - "If GPU memory exceeds 90%, consider running the memory_leak skill."
  - "Check the NCCL culprit/victim skill if collective times are high."
```

## SKILL.md

Agent-facing markdown for Cursor, Claude Code, and Codex integration. YAML
frontmatter is parsed by agent systems; the markdown body is human-readable.

```markdown
---
name: my_skill
description: >-
  Diagnose my specific issue.
category: performance
tables: [python.torch_trace, python.comm_collective]
tags: [gpu, memory, performance]
keywords:
  en: ['slow', 'bottleneck', 'throughput']
  zh: ['慢', '瓶颈', '吞吐']
parameters:
  step_window: { type: integer, default: 20 }
---

# My Skill

Detailed explanation of what this skill does, when to use it,
and how to interpret the results.
```

## Skill resolution and overlay

Skills are loaded from multiple roots in priority order (highest wins):

1. `$PROBING_SKILLS_DIR` — explicit extra root (when set)
2. `$PWD/.probing/skills/` — project overrides
3. `$HOME/.probing/skills/` — user overrides
4. Repo / wheel bundled skills (`skills/` at dev time; `python/probing/bundled_skills/` in the wheel)
5. Embedded defaults in the CLI binary (lowest)

Later roots override earlier ones for the same skill ID. The catalog (`catalog.yaml`)
is also merged across roots — entries in higher-priority roots replace lower-priority
entries with the same ID.

This means you can override a built-in skill by placing a modified copy in your
project's `.probing/skills/` directory.

## Installing skills for AI agents

```bash
# Install all skills for Cursor, Claude Code, and Codex
probing skill install

# Install specific agents only
probing skill install --agent cursor --agent claude

# Install to user-level agent directories only
probing skill install --user
```

This copies each skill's `SKILL.md` into `~/.cursor/skills/`,
`~/.claude/skills/`, or `~/.agents/skills/` so those agent tools can discover
and execute the skills during conversations.

## Validation

```bash
# Validate a single skill
python -m probing.skills validate my_skill

# Validate all skills
python -m probing.skills validate --all
```

The validator checks: missing steps, duplicate step IDs, read-only SQL compliance
(all statements must start with SELECT/WITH/SHOW/DESCRIBE), and missing SKILL.md.

# Diagnostic skills (authoring)

**Skill data lives here** (`python/probing/bundled_skills/`). Repo-root [`skills/`](../../../skills)
is a symlink to this directory for authoring ergonomics (`./skills/install.sh`, docs, L4 layout).

Each skill is a folder with **`SKILL.md`** (when/how, for agents) and optional **`steps.yaml`**
(deterministic steps for `probing skill run` and the Web Investigate agent).

## Related paths

| Path | What it is |
|------|------------|
| **`python/probing/bundled_skills/`** (here) | SSOT — packaged in wheel via maturin `include` |
| **`skills/`** (repo root) | Symlink → here |
| **`python/probing/skills/`** | Python **code** — loader, discovery, install (`probing.skills.*`) |
| **`examples/probing-acme/`** | Vendor extension template (`probing-<vendor>` entry points) |

## Layout

```
python/probing/bundled_skills/
├── catalog.yaml           # index (id, category, path)
├── semantic/              # intents, pages, table semantics (routing)
├── install.sh             # copy into .cursor/.claude/.agents skill dirs
└── health_overview/
    ├── SKILL.md
    └── steps.yaml
```

## Install into Cursor / Claude / Codex

```bash
./skills/install.sh
# or: probing skill install [--user] [--force]
```

| Agent | Project | User (`--user`) |
|-------|---------|-----------------|
| Cursor | `.cursor/skills/` | `~/.cursor/skills/` |
| Claude Code | `.claude/skills/` | `~/.claude/skills/` |
| Codex | `.agents/skills/` | `~/.agents/skills/` |

## Run

```bash
probing skill list
probing -t <pid> skill run health_overview
probing -t <pid> skill run slow_rank --set step_window=30 --global
```

Validate after edits:

```bash
python -m probing.skills validate
make wheel   # bundle web UI; skills ship from this directory
```

## Add a skill

1. `skills/my_skill/SKILL.md` — frontmatter with `name` + `description`
2. `skills/my_skill/steps.yaml` — optional executable steps
3. Register in `catalog.yaml`
4. `python -m probing.skills validate`
5. `./skills/install.sh` — refresh agent directories

See [AGENTS.md](../../../AGENTS.md), [docs/src/design/extensibility.md](../../../docs/src/design/extensibility.md), and vendor template [examples/probing-acme/](../../../examples/probing-acme/).

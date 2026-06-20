# `probing.skills` — Python runtime (not skill data)

**This package is code, not skill content.** Skill folders live at the repo root
[`skills/`](../../../skills/README.md). Wheel installs ship a copy under
[`../_skills/`](../_skills/README.md).

## Modules

| Module | Role |
|--------|------|
| `loader.py` | Load `catalog.yaml`, `steps.yaml`, semantic YAML; expand templates |
| `paths.py` | Discover skill roots (repo, bundled, user, project, `PROBING_SKILLS_DIR`) |
| `install.py` | `probing skill install` / `update` — copy into agent skill dirs |
| `tools.py` | `list_skills`, `run_skill`, JSON helpers for agents |
| `interpret.py` | Rule evaluation on step results |
| `__main__.py` | `python -m probing.skills validate\|install\|update` |

## Resolution order (later wins)

1. Bundled `python/probing/_skills/` (wheel)
2. Repo `skills/` (checkout)
3. `~/.probing/skills/`
4. `<project>/.probing/skills/`
5. `$PROBING_SKILLS_DIR`

## Public API

```python
from probing.skills.tools import list_skills, run_skill
from probing.skills.loader import load_skill, load_catalog
from probing.skills.install import install_skills
```

Rust CLI and Web UI embed the same catalog from repo `skills/` at compile time;
this package is used for Python tooling, install, and runtime overrides.

## Tests

`tests/unit/probing/skills/` — loader, interpret. `tests/regression/skills/` — install, tools.

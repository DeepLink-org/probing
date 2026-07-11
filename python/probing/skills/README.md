# `probing.skills` — Python runtime (not skill data)

**This package is code, not skill content.** Authoring tree: [`python/probing/bundled_skills/`](../bundled_skills/README.md)
(repo-root [`skills/`](../../../skills) is a symlink alias).

## Modules

| Module | Role |
|--------|------|
| `loader.py` | Thin wrappers over `probing._core.skills_*` (Rust SSOT) |
| `paths.py` | Agent install targets only; re-exports `skill_roots` from `probing.extensions` |
| `../extensions/` | **Extension discovery SSOT** — entry points, overlays, vendor inventory |
| `install.py` | `probing skill install` / `update` — copy into agent skill dirs |
| `tools.py` | `list_skills`, `plan_skill_run` — discovery/plan via Rust |
| `__main__.py` | `python -m probing.skills validate\|install\|update` |

## Discovery (later wins)

| Priority | Source | Mechanism |
|----------|--------|-----------|
| 1 | Bundled | `probing.skills` entry point `bundled`, or filesystem fallback (`PYTHONPATH` dev) |
| 2 | Repo | `skills/` in probing checkout |
| 3 | Installed packages | `probing.skills` entry points (`probing`, `probing-nvidia`, …) |
| 4 | User | `~/.probing/skills/` |
| 5 | Project | `<project>/.probing/skills/` |
| 6 | Env | `$PROBING_SKILLS_DIR` |

Vendor extensions (`probing-<vendor>`) register skills and magics via **the same entry-point groups**.
See [`docs/src/design/extensibility.md`](../../../docs/src/design/extensibility.md#path-4-vendor-extension-package-probing-vendor)
and template [`examples/probing-acme/`](../../../examples/probing-acme/).

```bash
python -m probing.extensions skill-roots
python -m probing.extensions extensions
```

Rust CLI and Web UI load skills at **runtime** from the probing server / Python discovery — not compile-time embed.

## Public API

```python
from probing.skills.tools import list_skills, plan_skill_run
from probing.skills.loader import load_skill, load_catalog
from probing.skills.install import install_skills
```

## Tests

`tests/unit/probing/skills/` — loader (Rust bridge). `tests/regression/skills/` — install, tools.
`tests/unit/probing/test_extensions.py` — entry-point discovery (legacy `test_discovery.py` covers shim).

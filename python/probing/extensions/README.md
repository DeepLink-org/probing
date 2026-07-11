# Extension discovery (`probing.extensions`)

**Single boundary** for pip-installed skills, REPL magics, and `probing-<vendor>`
packages. If you change how extensions are discovered or registered, edit **only**
this package.

## Layout

| Module | Responsibility |
|--------|----------------|
| `entrypoints.py` | Load `probing.skills` / `probing.magics` setuptools groups |
| `skills.py` | Merge skill roots (bundled, repo, entry points, user overrides) |
| `vendor.py` | `probing-<vendor>` naming and inventory |
| `types.py` | `SkillRoot` |
| `__main__.py` | `python -m probing.extensions {skill-roots\|extensions}` |

## Public API

```python
from probing.extensions import (
    skill_roots,
    skill_roots_json,
    load_magics,
    list_vendor_extensions,
    vendor_package_name,
)
```

## Consumers (thin — do not add discovery logic here)

| Consumer | Uses |
|----------|------|
| `probing.skills.loader` | `skill_roots()` via `paths` re-export |
| `probing.repl` | `load_magics()` |
| `probing.handlers.pythonext` | `skill_roots_json`, `vendor_extensions_json` |
| `probing/cli` (Rust) | subprocess `python -m probing.extensions skill-roots` |

## Authoring vendor packages

See `examples/probing-acme/` and `docs/src/design/extensibility.md`.

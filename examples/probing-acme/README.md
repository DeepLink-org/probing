# probing-acme — vendor extension template

Canonical layout for a **vendor extension package**. Real packages follow the same
shape with their vendor slug:

| Vendor | PyPI name | Import package |
|--------|-----------|----------------|
| NVIDIA | `probing-nvidia` | `probing_nvidia` |
| Huawei | `probing-huawei` | `probing_huawei` |
| Demo (this tree) | `probing-acme` | `probing_acme` |

## What it provides

Both skills and magics register via **setuptools entry points** (same mechanism):

| Group | Entry | Points to |
|-------|-------|-----------|
| `probing.skills` | `acme` | `skill_root()` → `probing_acme/skills/` |
| `probing.magics` | `acme` | `AcmeMagic` → REPL `%acme …` |

`[tool.setuptools.package-data]` only bundles `skills/**` into the wheel — probing
does not scan package data for discovery.

Optional: **table plugins** (`@table` + `python.enabled=probing_acme`).

## Developer mode (`pip install -e .`)

```bash
cd examples/probing-acme
pip install -e .          # registers entry points once

# Edit skills/ or magics.py — no rebuild needed
python -m probing.extensions extensions
probing skill list        # includes acme_health when probing is installed
```

With a probed training process, magics appear in the Python REPL (`%acme hello`).

## Authoring a real package

1. Copy this directory; rename `acme` → your vendor slug.
2. Use the **same slug** in `probing.skills` and `probing.magics` entry points.
3. Prefix skill ids and magic names (`nvidia_*`, `%nvidia_*`).
4. Publish to PyPI as `probing-<vendor>`.

See [Vendor extension packages](../../docs/src/design/extensibility.zh.md#path-4-vendor-extension-package-probing-vendor).

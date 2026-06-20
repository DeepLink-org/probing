# Installation

How to install Probing for **production use** or **wheel-based evaluation**.
Contributors building from a git checkout should follow [Contributing — Development setup](contributing.md#development-setup) (`make develop`), not this page alone.

## Requirements

| Component | Version |
|-----------|---------|
| Python | 3.7+ (3.9+ recommended for development) |
| Rust (source build only) | stable channel — see [Contributing — Prerequisites](contributing.md#prerequisites) |
| OS (full features) | Linux — required for `probing inject` |
| OS (in-process only) | macOS / Windows — `PROBING=1` at startup, query/eval via CLI |

## Install from PyPI (recommended)

```bash
pip install probing
# or: uv pip install probing
```

Verify:

```bash
probing --version
probing list
```

## Enable probing in your training job

After installation, the wheel ships a **site hook** (`probing.pth` → `probing_hook.py`) that can auto-import probing when the `PROBING` environment variable is set — no code changes required.

```bash
# Current process only
PROBING=1 python train.py

# Current process + child processes (torchrun, mp.spawn, …)
PROBING=2 python train.py
```

Common values:

| `PROBING` | Behavior |
|-----------|----------|
| unset / `0` | Disabled (default) |
| `1` / `followed` | Enable in current process |
| `2` / `nested` | Enable in current and child processes |
| `regex:PATTERN` | Enable when script name matches regex |
| `SCRIPT.py` | Enable when script basename matches |

Advanced filters and `init:…` prefixes: see `python/probing/site_hook.py` in the repository.

On **Linux**, you can also attach to a running process:

```bash
probing -t <pid> inject
```

On **macOS / Windows**, use `PROBING=1` (or `2`) at startup; injection is not available.

## Install from a release wheel (source build)

Use this for CI smoke tests or when you need a locally built wheel — **not** for day-to-day hacking on the repo (use `make develop` instead).

Requires **Rust stable** and optional frontend tools — see [Contributing — Prerequisites](contributing.md#prerequisites).

```bash
git clone https://github.com/DeepLink-org/probing.git
cd probing

# Install Rust stable if needed (see contributing.md#prerequisites)
make frontend
make wheel
pip install dist/probing-*.whl --force-reinstall
# or: make install-wheel
```

Verify as above. The installed wheel includes the same site hook as PyPI.

## Platform support

| Platform | `probing inject` | In-process (`PROBING=1`) | CLI query / eval |
|----------|------------------|---------------------------|------------------|
| Linux | ✅ | ✅ | ✅ |
| macOS | ❌ | ✅ | ✅ |
| Windows | ❌ | ✅ | ✅ |

## Upgrade

```bash
pip install --upgrade probing
```

See [Versions](versions.md) for Python / PyTorch / NCCL compatibility notes.

## Optional: example & ML dependencies

The core package has **no** hard Python dependencies. Repository examples under `examples/` may require extra packages (e.g. `torch`, `torchvision`). See [examples/README.md](https://github.com/DeepLink-org/probing/blob/main/examples/README.md).

## Next steps

- [Quick Start](quickstart.md)
- [Core Concepts](guide/concepts.md) — endpoints, in-process vs attach
- [Contributing](contributing.md) — clone, `make develop`, run tests

# Version Compatibility

This page documents Probing version compatibility. The version number below matches the
current release in this repository (`Cargo.toml` / `pyproject.toml`).

## Current Version

**Probing v0.2.5** (latest in this repo)

Check your install:

```bash
pip show probing
# or: python -c "import probing; print(probing.VERSION)"
```

## System Requirements

### Python

| Probing | Python |
|---------|--------|
| 0.2.x | 3.7 – 3.12 (see `pyproject.toml` classifiers) |

### PyTorch

Torch profiling (`python.torch_trace`, `python.comm_collective`) requires PyTorch when used;
no strict minimum is pinned in the package—use PyTorch 2.x for distributed training workflows.

### Operating Systems

| OS | Support |
|----|---------|
| **Linux** | Full support; `probing inject` (dynamic attach) is **Linux only** |
| **macOS** | Query / eval / in-process `PROBING=1`; inject not available |
| **Windows** | Experimental; WSL2 recommended for inject |

## Recent capabilities (0.2.x)

Documented features in this tree include:

- DataFusion SQL engine and `global.*` federated catalog
- `python.torch_trace`, `python.comm_collective`, `python.trace_event`
- Parallel **role** key on data rows + federation tag **`_role`**
- Runtime `probing.set_role()` / `current_role()` / `clear_role()`
- Diagnostic **skills** (`probing skill run …`, Web Agent)
- Optional NCCL profiler plugin (`nccl.proxy_ops`)

## Upgrade

```bash
pip install --upgrade probing
```

If you use torch profiling, prefer:

```bash
PROBING_TORCH_PROFILING=on python train.py
```

or `configure("on")` from `probing.profiling.torch_probe`.

## Reporting Issues

[GitHub Issues](https://github.com/DeepLink-org/probing/issues) — include `pip show probing`,
Python version, OS, and a minimal repro.

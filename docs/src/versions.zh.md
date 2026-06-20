# 版本兼容性

本页记录 Probing 版本与系统要求。下列版本号与当前仓库发布一致（`Cargo.toml` / `pyproject.toml`）。

## 当前版本

**Probing v0.2.5**（本仓库最新）

```bash
pip show probing
# 或: python -c "import probing; print(probing.VERSION)"
```

## 系统要求

### Python

| Probing | Python |
|---------|--------|
| 0.2.x | 3.7 – 3.12（见 `pyproject.toml`） |

### PyTorch

使用 `python.torch_trace`、`python.comm_collective` 等能力时需要 PyTorch；包内未钉死最低版本，分布式训练建议使用 PyTorch 2.x。

### 操作系统

| 系统 | 支持 |
|------|------|
| **Linux** | 完整支持；`probing inject` **仅 Linux** |
| **macOS** | query / eval / 进程内 `PROBING=1`；无 inject |
| **Windows** | 实验性；inject 建议 WSL2 |

## 0.2.x 主要能力

- DataFusion SQL 与 `global.*` 联邦 catalog
- `python.torch_trace`、`python.comm_collective`、`python.trace_event`
- 数据行 **role** + 联邦标签 **`_role`**
- `probing.set_role()` / `current_role()` / `clear_role()`
- 诊断 **skill**（`probing skill run …`、Web Agent）
- 可选 NCCL profiler 插件（`nccl.proxy_ops`）

## 升级

```bash
pip install --upgrade probing
```

Torch profiling 请使用 `PROBING_TORCH_PROFILING=on` 或 `configure("on")`（`probing.profiling.torch_probe`）。

## 反馈问题

[GitHub Issues](https://github.com/DeepLink-org/probing/issues) — 请附上版本、Python、操作系统与最小复现。

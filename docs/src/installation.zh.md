# 安装指南

本文说明如何**安装并使用** Probing（PyPI 或本地 wheel）。
在仓库里改代码、跑测试请直接看 [贡献指南 — 开发环境](contributing.zh.md#development-setup)，使用 `make develop`，不要只按本文装 wheel。

## 环境要求

| 组件 | 版本 |
|------|------|
| Python | 3.7+（开发建议 3.9+） |
| Rust（仅源码构建） | stable 通道 — 见 [贡献指南 — 前提条件](contributing.zh.md#prerequisites) |
| 操作系统（完整功能） | Linux — `probing inject` 需要 |
| 操作系统（仅进程内） | macOS / Windows — 启动时 `PROBING=1`，CLI query/eval 可用 |

## 从 PyPI 安装（推荐）

```bash
pip install probing
# 或：uv pip install probing
```

验证：

```bash
probing --version
probing list
```

## 在训练任务中启用 probing

安装后，wheel 自带 **site hook**（`probing.pth` → `probing_hook.py`）：设置 `PROBING` 环境变量即可自动 import probing，**无需改训练脚本**。

```bash
# 仅当前进程
PROBING=1 python train.py

# 当前进程 + 子进程（torchrun、mp.spawn 等）
PROBING=2 python train.py
```

常用取值：

| `PROBING` | 行为 |
|-----------|------|
| 未设置 / `0` | 关闭（默认） |
| `1` / `followed` | 当前进程启用 |
| `2` / `nested` | 当前进程及子进程启用 |
| `regex:PATTERN` | 脚本名匹配正则时启用 |
| `SCRIPT.py` | 脚本 basename 完全匹配时启用 |

高级过滤与 `init:…` 前缀见仓库内 `python/probing/site_hook.py`。

在 **Linux** 上还可 attach 已运行进程：

```bash
probing -t <pid> inject
```

**macOS / Windows** 请在启动时设置 `PROBING=1`（或 `2`）；不支持 inject。

## 从源码构建 wheel 安装

适用于 CI 冒烟或本地验 wheel — **不是**日常改仓库代码的方式（请用 `make develop`）。

需要 **Rust stable** 及可选的前端工具链 — 见 [贡献指南 — 前提条件](contributing.zh.md#prerequisites)。

```bash
git clone https://github.com/DeepLink-org/probing.git
cd probing

# 若尚未安装 Rust stable，见 contributing.zh.md#prerequisites
make frontend
make wheel
pip install dist/probing-*.whl --force-reinstall
# 或：make install-wheel
```

验证方式同上。本地 wheel 与 PyPI 一样包含 site hook。

## 平台支持

| 平台 | `probing inject` | 进程内（`PROBING=1`） | CLI query / eval |
|------|------------------|----------------------|------------------|
| Linux | ✅ | ✅ | ✅ |
| macOS | ❌ | ✅ | ✅ |
| Windows | ❌ | ✅ | ✅ |

## 升级

```bash
pip install --upgrade probing
```

版本与 PyTorch / NCCL 兼容性见 [版本说明](versions.zh.md)。

## 可选：示例与 ML 依赖

核心包**无**强制 Python 依赖。`examples/` 下的示例可能需要额外安装（如 `torch`、`torchvision`），见 [examples/README.md](https://github.com/DeepLink-org/probing/blob/main/examples/README.md)。

## 下一步

- [快速开始](quickstart.zh.md)
- [核心概念](guide/concepts.zh.md) — endpoint、进程内 vs attach
- [贡献指南](contributing.zh.md) — 克隆、`make develop`、测试

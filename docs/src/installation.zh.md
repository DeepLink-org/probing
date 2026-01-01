# 安装指南

本指南介绍如何在您的系统上安装 Probing。

## 环境要求

在开始之前，请确保您的系统满足以下要求：

- Python（3.7 或更高版本）
- Pip（Python 包安装器）
- 如需从源码构建：
    - Rust（推荐最新稳定版）
    - Cargo（Rust 的包管理器和构建系统）

## 安装方式

### 1. 使用 Pip（推荐）

这是安装 Probing 最简单的方式：

```bash
pip install probing
```

此命令将从 Python Package Index (PyPI) 下载并安装 Probing 的最新稳定版本。

### 2. 从源码构建

如果您需要最新的开发版本或想要贡献代码，可以从源码构建：

```bash
# 1. 克隆仓库
git clone https://github.com/DeepLink-org/probing.git
cd probing

# 2. 构建并安装 Python 包
make wheel
pip install dist/probing-*.whl
```

这将编译 Rust 组件并构建用于安装的 Python wheel 包。

## 验证安装

安装完成后，可以通过以下命令验证 Probing 是否正确安装：

```bash
probing --version
```

应该会输出已安装的 Probing 版本，例如：

```
probing 0.2.3
```

您也可以检查 `probing` 命令是否可用：

```bash
probing list
```

此命令应该会列出可用的 probing 命令或显示当前没有进程正在被探测。

## 平台支持

| 平台 | 注入功能 | 查询/执行 |
|------|----------|-----------|
| Linux | ✅ 完全支持 | ✅ 完全支持 |
| macOS | ❌ 不支持 | ✅ 支持 |
| Windows | ❌ 不支持 | ✅ 支持 |

!!! note "注入功能需要 Linux"
    动态探针注入功能（`probing inject`）需要 Linux 系统。在其他平台上，如果目标进程在启动时启用了 probing，您仍然可以使用查询和执行功能。

## 下一步

安装完成后，您可以开始使用 Probing：

- [快速开始](quickstart.zh.md) - 开始您的第一次分析
- [SQL 分析](guide/sql-analytics.zh.md) - 学习 SQL 查询接口
- [内存分析](guide/memory-analysis.zh.md) - 调试内存问题

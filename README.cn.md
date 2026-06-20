# Probing - AI应用性能分析和调试工具

<div align="center">
  <img src="probing.svg" alt="Probing Logo" width="200"/>

  <p>
    <a href="README.cn.md">中文</a> |
    <a href="README.md">English</a>
  </p>
</div>

[![PyPI version](https://badge.fury.io/py/probing.svg)](https://badge.fury.io/py/probing)
[![License](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](https://www.apache.org/licenses/LICENSE-2.0)
[![Downloads](https://pepy.tech/badge/probing)](https://pepy.tech/project/probing)
[![codecov](https://codecov.io/gh/DeepLink-org/probing/graph/badge.svg?token=IRH3F0OI56)](https://codecov.io/gh/DeepLink-org/probing)

> 揭开AI应用性能的真相

Probing是一个专为AI应用设计的运行时性能分析和调试工具。基于动态探针注入技术，它提供零侵入的运行时内省能力，支持SQL查询的性能指标和跨节点关联分析。

## Probing 提供什么...

### 🔍 **面向AI研究员和算法工程师**
- **调试训练不稳定性** - 实时洞察训练为何发散或卡住
- **优化模型性能** - 识别前向/反向传播中的瓶颈
- **内存泄漏检测** - 跟踪训练步骤间的GPU/CPU内存使用
- **实时变量检查** - 在不停止训练的情况下检查张量值、梯度和模型状态

### 🛠️ **面向框架和库开发者**
- **运行时框架分析** - 了解框架在真实使用场景中的表现
- **零侵入性能分析** - 无需代码修改即可分析框架内部
- **生产环境调试** - 在用户实际环境中调试报告的问题
- **性能基准测试** - 收集真实性能数据用于优化决策

### ⚙️ **面向系统工程师和MLOps**
- **生产环境监控** - 无需重启服务即可监控AI应用
- **资源优化** - 分析集群中的资源使用模式
- **自定义指标收集** - 收集任意应用特定的性能数据
- **分布式调试** - 跨多个节点关联性能问题

### 🚀 **核心技术能力**
- **动态探针注入** - 无需代码修改即可附加到运行中的进程
- **SQL驱动的分析** - 使用标准SQL查询性能数据
- **实时代码执行** - 直接在目标进程中运行Python代码
- **实时堆栈分析** - 捕获带变量值的执行上下文

### 与传统分析工具相比，Probing不会...

- **要求代码插桩** - 无需添加日志语句、插入计时器或修改训练脚本
- **强制"先中断再修复"工作流** - 无需等待问题发生，然后花费数天尝试复现
- **锁定在固定报告中** - 不再需要解析预格式化的表格；使用SQL创建符合特定需求的自定义分析报告
- **中断工作流程** - 无需停止训练作业或服务即可附加到运行中的进程
- **强迫学习新工具** - 使用熟悉的SQL语法和Python代码满足所有分析需求

## 快速开始

### 安装

```bash
pip install probing
```

### 快速开始 (30秒)

```bash
# 找到你的训练进程
export ENDPOINT=$(pgrep -f "python.*train")

# 注入探针
probing $ENDPOINT inject

# 检查当前执行状态
probing $ENDPOINT backtrace

# 执行Python代码检查状态
probing $ENDPOINT eval "import torch; print(f'GPU available: {torch.cuda.is_available()}')"

# 查询性能数据
probing $ENDPOINT query "SELECT func, file, lineno FROM python.backtrace LIMIT 5"
```

## 核心功能

- **动态探针注入** - 运行时检测，无需目标应用修改
- **SQL分析接口** - Apache DataFusion驱动的查询引擎，支持标准SQL语法
- **交互式Python REPL** - 运行中进程的实时调试和变量检查
- **实时内省** - 实时性能指标和运行时堆栈跟踪分析
- **高级CLI** - 全面的命令行接口，支持进程监控和管理

## 基本用法

```bash
# 注入性能监控
probing -t <pid> inject

# 实时堆栈跟踪分析
probing -t <pid> backtrace

# 执行Python代码
probing -t <pid> eval "import threading; print([t.name for t in threading.enumerate()])"

# SQL查询分析
probing -t <pid> query "SELECT * FROM python.backtrace ORDER BY depth LIMIT 10"

# 交互式Python REPL
probing -t <pid> repl

# 配置管理
probing -t <pid> config
```

## 高级功能

### SQL分析接口
```bash
# 内存使用分析
probing -t <pid> query "SELECT * FROM memory_usage WHERE timestamp > now() - interval '5 min'"

# 性能热点分析
probing -t <pid> query "
  SELECT operation_name, avg(duration_ms), count(*)
  FROM profiling_data
  WHERE timestamp > now() - interval '5 minutes'
  GROUP BY operation_name
  ORDER BY avg(duration_ms) DESC
"

# 训练进度跟踪
probing -t <pid> query "
  SELECT epoch, avg(loss), min(loss), count(*) as steps
  FROM training_logs
  GROUP BY epoch
  ORDER BY epoch
"
```

### 交互式Python REPL

Probing提供交互式Python REPL，可以连接到运行中的进程，允许您检查变量、执行代码和实时调试：

```bash
# 通过REPL连接到进程
probing -t <pid> repl

# 对于远程进程
probing -t <host|ip:port> repl
```

REPL会话示例：
```python
>>> import torch
>>> # 检查目标进程中的torch模型
>>> models = [m for m in gc.get_objects() if isinstance(m, torch.nn.Module)]
```

REPL提供：
- **实时变量检查**：访问目标进程上下文中的所有变量
- **代码执行**：在目标进程中运行任意Python代码
- **实时调试**：设置断点并检查状态，无需停止进程

### 配置选项
```bash
# 环境变量配置
export PROBING_SAMPLE_RATE=0.1      # 设置采样率
export PROBING_RETENTION_DAYS=7     # 数据保留期

# 查看当前配置
probing -t <pid> config

# 动态配置更新
probing -t <pid> config probing.sample_rate=0.05
probing -t <pid> config probing.max_memory=1GB
```

## 开发

**使用者：** `pip install probing` — 见 [安装指南](docs/src/installation.zh.md)。

**贡献者** — 初次参与？请先读 **[贡献指南 → 欢迎参与开发](docs/src/contributing.zh.md#getting-started)**（English: [Contributing](docs/src/contributing.md#getting-started)），然后：

```bash
python3 -m venv .venv && source .venv/bin/activate
pip install maturin
make develop
make check-dev
make test
```

| 文档 | 内容 |
|------|------|
| [贡献指南 — 欢迎](docs/src/contributing.zh.md#getting-started) | 选方向（skill / Python / 文档 / Rust / web）、第一个 PR |
| [安装指南](docs/src/installation.zh.md) | PyPI、wheel、`PROBING=1`、平台支持 |
| [贡献指南 — 开发环境](docs/src/contributing.zh.md#development-setup) | `make develop`、`.pth` hook、Makefile |
| [examples/README.md](examples/README.md) | 示例所需的 torch/torchvision 等 |

发布 wheel：`make wheel && make install-wheel`。

## 许可证

[Apache License 2.0](LICENSE)

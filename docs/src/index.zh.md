---
template: home.html
title: Probing - 分布式 AI 动态性能分析器
description: 专为分布式 AI 工作负载设计的生产级性能分析器。零侵入、SQL 驱动分析、实时内省。
hide: toc
---

<!-- 此内容被 home.html 模板隐藏，但会被搜索索引 -->

# Probing

**Probing** 是一个面向分布式 AI 应用的动态性能分析器。

## 核心特性

- **零侵入** - 无需修改代码即可附加到运行中的进程
- **SQL 分析** - 使用标准 SQL 查询性能数据
- **实时执行** - 在目标进程中运行 Python 代码
- **堆栈分析** - 捕获带有变量值的调用栈
- **分布式支持** - 监控跨多节点的进程

## 快速开始

```bash
# 安装
pip install probing

# 注入到运行中的进程
probing -t <pid> inject

# 查询性能数据
probing -t <pid> query "SELECT * FROM python.torch_trace LIMIT 10"
```

## 使用场景

- **训练调试** - 调试训练不稳定和卡住问题
- **内存分析** - 追踪 GPU/CPU 内存使用
- **性能分析** - 识别模型执行中的瓶颈
- **生产监控** - 无需重启即可监控 AI 服务

## 社区

- [GitHub 仓库](https://github.com/DeepLink-org/probing)
- [问题追踪](https://github.com/DeepLink-org/probing/issues)
- [PyPI 包](https://pypi.org/project/probing/)

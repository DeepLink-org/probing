# 设计概览

## 为什么选择 Probing？

### Pythonic 的优势

Python 在 AI 领域的主导地位源于一个核心原则：**一切都像 Python**。无论您使用 pandas、PyTorch 还是 NumPy，都可以用 **Pythonic 的方式与它们交互**——相同的 `print()`、迭代和属性访问模式随处可用。

### 分布式系统如何破坏了这一点

当 AI 模型扩展到分布式集群时，一些根本性的东西被打破了：**分布式系统不是 Pythonic 的**。单机调试感觉很自然——`print(model.parameters())`、`loss.item()`、`torch.cuda.memory_allocated()`——但分布式调试迫使您使用系统管理工具：`kubectl get nodes`、SSH 会话、日志文件解析、监控仪表板。

### Probing 的使命

Probing 的核心使命很简单：**让分布式系统重新变得 Pythonic**。您的集群、节点和分布式进程都可以通过熟悉的接口访问。不再需要在工具之间切换，您可以留在 Python 中，**用 Pythonic 的方式与您的分布式系统对话**。

## 设计原则

### 🔍 零侵入

- 无需修改代码
- 无需更改环境设置
- 无需中断工作流
- 动态探针注入到运行中的进程

### 🎯 零学习曲线

- 标准 SQL 接口用于数据分析
- 熟悉的数据库查询模式
- 直观的命令行工具
- 基于 Web 的可视化仪表板

### 📦 零部署负担

- 单一二进制部署（基于 Rust）
- 静态编译，最小依赖
- Linux 优先设计，其他平台支持查询/执行
- 弹性扩展能力

## 设计文档

| 文档 | 描述 |
|------|------|
| [系统架构](architecture.md) | 系统结构和组件 |
| [性能分析](profiling.md) | 性能数据收集 |
| [调试](debugging.md) | 调试能力 |
| [分布式](distributed.md) | 多节点支持 |
| [扩展机制](extensibility.md) | 自定义表和指标 |

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

与实现文档共用的术语（端点、step、role、联邦）：**[核心概念](../guide/concepts.zh.md)**。

| 文档 | 描述 |
|------|------|
| [模块化与边界](modularity.zh.md) | **核心 vs 功能模块**、接口、依赖规则、ownership |
| [系统架构](architecture.zh.md) | 系统结构和组件 |
| [数据层](data-layer.zh.md) | 冷热分层列式存储与 SQL 集成 |
| [性能分析](profiling.zh.md) | 性能数据收集 |
| [调试](debugging.zh.md) | 调试能力 |
| [分布式](distributed.zh.md) | 多节点支持 |
| [torchrun 集群心跳](torchrun-cluster.zh.md) | 分层注册、心跳退避、env 预设 |
| [联邦查询引擎](federation.zh.md) | 跨 rank SQL：诊断场景、三条执行路径、万卡验收 |
| [NCCL Profiler](nccl-profiler.zh.md) | NCCL 插件、culprit/victim、`nccl.proxy_ops` |
| [基于 Pulsing 的集群](cluster-pulsing.zh.md) | 使用 Pulsing 做成员发现与故障检测 |
| [扩展机制](extensibility.zh.md) | 自定义表和指标 |

用户向操作：**[用户指南](../guide/index.zh.md)**（SQL、skill、调试）。
参考：**[SQL 表目录](../reference/sql-tables.zh.md)** · **[API 参考](../api-reference.zh.md)**。

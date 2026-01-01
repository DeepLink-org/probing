# 用户指南

欢迎阅读 Probing 用户指南。本章节介绍核心功能和使用模式。

## 概览

Probing 提供三个核心能力来分析和调试您的 AI 应用：

| 能力 | 命令 | 描述 |
|------|------|------|
| **eval** | `probing $ENDPOINT eval "..."` | 在目标进程中执行 Python 代码 |
| **query** | `probing $ENDPOINT query "..."` | 使用 SQL 查询性能数据 |
| **backtrace** | `probing $ENDPOINT backtrace` | 捕获带变量的执行堆栈 |

## 入门指南

如果您是 Probing 新用户，建议按以下顺序阅读这些指南：

1. **[SQL 分析](sql-analytics.zh.md)** - 学习强大的 SQL 查询接口
2. **[内存分析](memory-analysis.zh.md)** - 调试内存泄漏和使用模式
3. **[调试指南](debugging.zh.md)** - 掌握堆栈分析和实时调试
4. **[常见问题](troubleshooting.zh.md)** - 常见问题和解决方案

## 核心概念

### 目标端点

所有 Probing 命令都需要一个目标端点，可以是：

- **进程 ID**：本地进程（如 `12345`）
- **远程地址**：网络端点（如 `host:8080`）

```bash
# 设置目标端点
export ENDPOINT=12345  # 或 host:8080
```

### 数据表

Probing 通过 SQL 表暴露性能数据：

| 表名 | 描述 |
|------|------|
| `python.backtrace` | 堆栈跟踪信息 |
| `python.torch_trace` | PyTorch 执行跟踪 |
| `python.variables` | 变量追踪 |
| `information_schema.df_settings` | 配置设置 |

### 工作流模式

**调试工作流：**
```bash
# 1. 捕获当前状态
probing $ENDPOINT backtrace

# 2. 检查特定值
probing $ENDPOINT eval "print(my_variable)"

# 3. 查询历史数据
probing $ENDPOINT query "SELECT * FROM python.torch_trace"
```

## 进阶主题

- **[系统架构](../design/architecture.md)** - 系统设计和内部实现
- **[分布式](../design/distributed.md)** - 多节点监控
- **[扩展机制](../design/extensibility.md)** - 自定义表和指标

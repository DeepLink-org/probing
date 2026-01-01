# 快速开始

通过这个精简的工作流程，快速获得 Probing 的价值。

## 5 分钟上手

### 步骤 1：设置目标进程

所有 Probing 命令都需要一个目标端点。将 `$ENDPOINT` 设置为本地进程 ID 或远程地址：

```bash
# 本地进程 - 查找并设置 Python 进程 ID
export ENDPOINT=$(pgrep -f "python.*your_script")

# 或者远程进程
export ENDPOINT=remote-host:8080
```

!!! tip "查找进程"
    使用 `ps aux | grep python` 或 `pgrep -f "python.*train"` 来定位目标进程。

### 步骤 2：连接并探索

```bash
# 连接到进程（仅 Linux）
probing $ENDPOINT inject

# 获取基本进程信息
probing $ENDPOINT eval "import os, psutil; proc = psutil.Process(); print(f'PID: {os.getpid()}, 内存: {proc.memory_info().rss/1024**2:.1f}MB')"
```

### 步骤 3：尝试三大核心功能

#### 📊 查询结构化数据

```bash
probing $ENDPOINT query "SELECT name, value FROM information_schema.df_settings LIMIT 5"
```

#### 🎯 执行实时代码

```bash
probing $ENDPOINT eval "import torch; print(f'CUDA: {torch.cuda.is_available()}')"
```

#### 🔍 捕获执行上下文

```bash
probing $ENDPOINT backtrace

probing $ENDPOINT query "SELECT func, file, lineno FROM python.backtrace ORDER BY depth LIMIT 5"
```

## 三大核心能力

Probing 提供三个强大的能力，它们协同工作：

### 🎯 eval：在运行中的进程执行代码

直接在目标进程中运行任意 Python 代码：

```bash
# 检查训练线程
probing $ENDPOINT eval "import threading; [print(f'{t.name}: {t.is_alive()}') for t in threading.enumerate()]"

# 检查 GPU 内存使用
probing $ENDPOINT eval "import torch; print(f'GPU: {torch.cuda.memory_allocated()/1024**3:.1f}GB 已分配')"
```

### 📊 query：用 SQL 分析数据

使用熟悉的 SQL 语法查询结构化性能数据：

```bash
probing $ENDPOINT query "
SELECT
    step,
    module,
    SUM(allocated) as total_memory_mb,
    COUNT(*) as operation_count
FROM python.torch_trace
WHERE step > 100
GROUP BY step, module
ORDER BY total_memory_mb DESC
LIMIT 10"
```

### 🔍 backtrace：带堆栈上下文的调试

捕获带有 Python 变量值的详细调用栈：

```bash
# 捕获当前调用栈
probing $ENDPOINT backtrace

# 查询堆栈跟踪
probing $ENDPOINT query "SELECT func, file, lineno FROM python.backtrace ORDER BY depth LIMIT 3"
```

## 真实调试场景

### 场景 1：训练进程卡住

**问题**：PyTorch 训练突然停止进展。

```bash
# 1. 查看主线程在做什么
probing $ENDPOINT backtrace

# 2. 检查线程状态
probing $ENDPOINT eval "import threading; [(t.name, t.is_alive()) for t in threading.enumerate()]"

# 3. 分析堆栈上下文
probing $ENDPOINT query "SELECT func, file, lineno FROM python.backtrace ORDER BY depth LIMIT 10"
```

### 场景 2：内存泄漏排查

**问题**：训练过程中内存使用持续增长。

```bash
# 强制清理并获取当前状态
probing $ENDPOINT eval "import gc, torch; gc.collect(); torch.cuda.empty_cache()"

# 分析分配趋势
probing $ENDPOINT query "SELECT step, AVG(allocated) as avg_memory FROM python.torch_trace GROUP BY step ORDER BY step"
```

### 场景 3：性能瓶颈分析

**问题**：需要找出哪些模型组件最慢。

```bash
# 查找最耗时的操作
probing $ENDPOINT query "
SELECT module, stage, AVG(duration) as avg_duration
FROM python.torch_trace
GROUP BY module, stage
ORDER BY avg_duration DESC
LIMIT 10"
```

## 下一步

- [SQL 分析](guide/sql-analytics.zh.md) - 高级查询技巧
- [内存分析](guide/memory-analysis.zh.md) - 深入内存调试
- [调试指南](guide/debugging.zh.md) - 专家级调试模式

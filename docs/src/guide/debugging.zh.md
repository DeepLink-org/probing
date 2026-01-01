# 调试指南

掌握使用 Probing 强大的内省能力调试 AI 应用的艺术。

## 概览

Probing 提供三种互补的调试方式：

- **backtrace**：捕获带有堆栈帧的执行上下文
- **eval**：在目标进程中执行任意 Python 代码
- **query**：使用 SQL 分析收集的数据

## 堆栈分析

### 捕获当前堆栈

```bash
# 获取当前执行堆栈
probing $ENDPOINT backtrace

# 查询堆栈帧
probing $ENDPOINT query "
SELECT func, file, lineno, depth
FROM python.backtrace
ORDER BY depth
LIMIT 10"
```

### 理解堆栈帧

每个堆栈帧包括：

| 字段 | 描述 |
|------|------|
| func | 函数名 |
| file | 源文件路径 |
| lineno | 行号 |
| depth | 堆栈深度（0 = 最内层）|
| frame_type | Python 或 Native |

### 找到代码卡住的位置

```bash
# 捕获堆栈
probing $ENDPOINT backtrace

# 检查堆栈顶部
probing $ENDPOINT query "
SELECT func, file, lineno
FROM python.backtrace
WHERE depth = 0"
```

## 实时检查

### 检查变量

```bash
# 检查全局变量
probing $ENDPOINT eval "print(globals().keys())"

# 检查特定对象
probing $ENDPOINT eval "print(type(model), model)"

# 检查模型参数
probing $ENDPOINT eval "
for name, param in model.named_parameters():
    print(f'{name}: {param.shape}, grad={param.grad is not None}')
"
```

### 检查线程状态

```bash
probing $ENDPOINT eval "
import threading
for t in threading.enumerate():
    print(f'{t.name}: alive={t.is_alive()}, daemon={t.daemon}')
"
```

### 监控训练进度

```bash
probing $ENDPOINT eval "
# 检查当前步骤
print(f'当前步骤: {trainer.current_step}')
print(f'当前损失: {trainer.last_loss}')
print(f'学习率: {optimizer.param_groups[0][\"lr\"]}')
"
```

## 调试场景

### 场景 1：训练卡住

**症状**：训练进度停止，没有错误。

**诊断**：

```bash
# 1. 捕获堆栈
probing $ENDPOINT backtrace

# 2. 检查是什么阻塞了
probing $ENDPOINT query "
SELECT func, file, lineno
FROM python.backtrace
WHERE depth < 5"

# 3. 检查线程状态
probing $ENDPOINT eval "
import threading
for t in threading.enumerate():
    print(f'{t.name}: {t.is_alive()}')"

# 4. 检查死锁
probing $ENDPOINT eval "
import torch.distributed as dist
if dist.is_initialized():
    print(f'Rank: {dist.get_rank()}, World: {dist.get_world_size()}')"
```

### 场景 2：梯度中出现 NaN/Inf

**症状**：损失变成 NaN 或 Inf。

**诊断**：

```bash
# 检查模型参数中的 NaN
probing $ENDPOINT eval "
import torch
for name, param in model.named_parameters():
    if param.grad is not None:
        if torch.isnan(param.grad).any():
            print(f'{name} 梯度中有 NaN')
        if torch.isinf(param.grad).any():
            print(f'{name} 梯度中有 Inf')"

# 检查损失值
probing $ENDPOINT eval "
import torch
print(f'损失: {loss.item()}')
print(f'是否 NaN: {torch.isnan(loss).item()}')
print(f'是否 Inf: {torch.isinf(loss).item()}')"
```

### 场景 3：训练慢

**症状**：训练比预期慢。

**诊断**：

```sql
-- 找到最慢的操作
SELECT module, stage, avg(duration) as avg_time
FROM python.torch_trace
WHERE step > (SELECT max(step) - 5 FROM python.torch_trace)
GROUP BY module, stage
ORDER BY avg_time DESC
LIMIT 10;
```

## 最佳实践

1. **先用 backtrace** - 在深入之前先了解执行位置
2. **用 query 分析趋势** - SQL 非常适合分析时间模式
3. **用 eval 获取实时状态** - 用 Python 代码获取当前状态
4. **组合使用** - backtrace → eval → query 工作流
5. **记录重要状态** - 使用 eval 打印诊断信息

## 下一步

- [内存分析](memory-analysis.zh.md) - 调试内存问题
- [常见问题](troubleshooting.zh.md) - 常见问题和解决方案
- [系统架构](../design/architecture.md) - 了解内部实现

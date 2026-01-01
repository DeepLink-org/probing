# 内存泄漏示例

检测和修复 AI 应用中的内存泄漏。

## 识别内存泄漏

### 内存增长模式

```bash
# 追踪各步骤的内存
probing $ENDPOINT query "
SELECT
    step,
    MAX(allocated) as peak_memory_mb
FROM python.torch_trace
GROUP BY step
ORDER BY step"
```

### 单调增长检测

```sql
WITH memory_trend AS (
  SELECT
    step,
    MAX(allocated) as peak,
    LAG(MAX(allocated)) OVER (ORDER BY step) as prev_peak
  FROM python.torch_trace
  GROUP BY step
)
SELECT
    step,
    peak,
    peak - prev_peak as growth
FROM memory_trend
WHERE peak > prev_peak
ORDER BY step;
```

## 常见泄漏模式

### 模式 1：列表中累积张量

**问题：**

```python
# 错误：张量在列表中累积
all_losses = []
for batch in dataloader:
    loss = model(batch)
    all_losses.append(loss)  # 保持计算图！
```

**检测：**

```bash
probing $ENDPOINT eval "
import gc
import torch
tensors = [obj for obj in gc.get_objects() if isinstance(obj, torch.Tensor)]
print(f'张量总数: {len(tensors)}')"
```

**修复：**

```bash
probing $ENDPOINT eval "
# 使用 .item() 提取标量
all_losses.clear()
print('已清空损失列表')"
```

### 模式 2：遗忘的梯度图

**问题：**

```python
# 错误：中间张量保持 grad_fn
intermediate = model.encoder(x)
# ... 很多操作 ...
# intermediate 仍然保持计算图
```

**检测：**

```bash
probing $ENDPOINT eval "
import torch
for name, param in model.named_parameters():
    if param.grad is not None:
        print(f'{name}: grad_fn={param.grad.grad_fn}')"
```

**修复：**

```bash
probing $ENDPOINT eval "
model.zero_grad(set_to_none=True)
import torch
torch.cuda.empty_cache()
print('已清除梯度')"
```

### 模式 3：循环引用

**检测：**

```bash
probing $ENDPOINT eval "
import gc
gc.set_debug(gc.DEBUG_SAVEALL)
gc.collect()
print(f'无法回收: {len(gc.garbage)}')"
```

## GPU 内存泄漏

### 检查 CUDA 内存状态

```bash
probing $ENDPOINT eval "
import torch
print(f'已分配: {torch.cuda.memory_allocated() / 1024**3:.2f} GB')
print(f'已保留: {torch.cuda.memory_reserved() / 1024**3:.2f} GB')
print(f'峰值: {torch.cuda.max_memory_allocated() / 1024**3:.2f} GB')"
```

### 内存快照

```bash
probing $ENDPOINT eval "
import torch
if torch.cuda.is_available():
    snapshot = torch.cuda.memory_snapshot()
    print(f'块数量: {len(snapshot)}')"
```

### 强制 CUDA 清理

```bash
probing $ENDPOINT eval "
import torch
import gc

# 清除所有引用
gc.collect()

# 清空 CUDA 缓存
torch.cuda.empty_cache()

# 重置峰值统计
torch.cuda.reset_peak_memory_stats()

print('CUDA 内存已清理')"
```

## CPU 内存泄漏

### 追踪进程内存

```bash
probing $ENDPOINT eval "
import psutil
import os

proc = psutil.Process(os.getpid())
mem = proc.memory_info()
print(f'RSS: {mem.rss / 1024**3:.2f} GB')
print(f'VMS: {mem.vms / 1024**3:.2f} GB')"
```

### 查找大对象

```bash
probing $ENDPOINT eval "
import sys
import gc

# 查找最大对象
objects = gc.get_objects()
sizes = [(sys.getsizeof(obj), type(obj).__name__) for obj in objects[:1000]]
sizes.sort(reverse=True)
for size, name in sizes[:10]:
    print(f'{name}: {size / 1024:.1f} KB')"
```

## 数据加载器泄漏

### 检查 Worker 状态

```bash
probing $ENDPOINT eval "
print(f'Num workers: {train_loader.num_workers}')
print(f'Batch size: {train_loader.batch_size}')"
```

### 检查预取数据

```bash
probing $ENDPOINT eval "
# 检查数据是否被保持
if hasattr(train_loader, '_iterator'):
    print('迭代器存在')
else:
    print('没有活动的迭代器')"
```

## 持续监控

### 定期内存检查

```bash
# 每分钟运行
while true; do
    probing $ENDPOINT eval "
import torch
import psutil
import os
proc = psutil.Process(os.getpid())
print(f'CPU: {proc.memory_info().rss / 1024**3:.2f} GB, GPU: {torch.cuda.memory_allocated() / 1024**3:.2f} GB')"
    sleep 60
done
```

### 基于 SQL 的监控

```sql
-- 最近 100 步的内存趋势
SELECT
    step,
    AVG(allocated) as avg_memory,
    MAX(allocated) as peak_memory
FROM python.torch_trace
WHERE step > (SELECT MAX(step) - 100 FROM python.torch_trace)
GROUP BY step
ORDER BY step;
```

## 预防最佳实践

### 1. 使用上下文管理器

```python
# 好的做法
with torch.no_grad():
    output = model(input)
```

### 2. 分离张量

```python
# 存储时
stored_output = output.detach().cpu()
```

### 3. 定期清理缓存

```python
if step % 100 == 0:
    gc.collect()
    torch.cuda.empty_cache()
```

### 4. 使用 `set_to_none=True`

```python
optimizer.zero_grad(set_to_none=True)  # 更高效
```

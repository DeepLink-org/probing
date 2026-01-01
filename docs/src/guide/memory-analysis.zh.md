# 内存分析

Probing 提供全面的工具用于分析 AI 应用中的内存使用。

## 概览

内存问题在 AI 工作负载中很常见，特别是在训练期间。Probing 帮助您：

- 追踪 GPU 和 CPU 内存分配
- 检测内存泄漏
- 分析内存使用模式
- 优化内存效率

## 快速内存检查

```bash
# 获取当前内存状态
probing $ENDPOINT eval "
import torch
import psutil

proc = psutil.Process()
print(f'CPU 内存: {proc.memory_info().rss / 1024**3:.2f} GB')

if torch.cuda.is_available():
    print(f'GPU 已分配: {torch.cuda.memory_allocated() / 1024**3:.2f} GB')
    print(f'GPU 缓存: {torch.cuda.memory_reserved() / 1024**3:.2f} GB')
"
```

## 内存使用趋势

### 追踪训练步骤的内存变化

```sql
SELECT
  step,
  avg(allocated) as avg_memory_mb,
  max(allocated) as peak_memory_mb,
  min(allocated) as min_memory_mb
FROM python.torch_trace
WHERE step IS NOT NULL
GROUP BY step
ORDER BY step;
```

### 检测内存增长

```sql
SELECT
  step,
  max(allocated) - min(allocated) as memory_growth_mb
FROM python.torch_trace
WHERE step > (SELECT max(step) - 10 FROM python.torch_trace)
GROUP BY step
HAVING max(allocated) - min(allocated) > 50
ORDER BY step;
```

## 按模块分析内存

识别哪些模型组件使用最多内存：

```sql
SELECT
  module,
  stage,
  avg(allocated) as avg_memory,
  count(*) as execution_count
FROM python.torch_trace
WHERE module IS NOT NULL
GROUP BY module, stage
ORDER BY avg_memory DESC
LIMIT 10;
```

## 内存泄漏检测

### 模式：单调增长

```sql
WITH memory_trend AS (
  SELECT
    step,
    max(allocated) as peak_memory,
    LAG(max(allocated)) OVER (ORDER BY step) as prev_peak
  FROM python.torch_trace
  GROUP BY step
)
SELECT step, peak_memory, prev_peak,
       peak_memory - prev_peak as growth
FROM memory_trend
WHERE peak_memory > prev_peak
ORDER BY step;
```

### 强制垃圾回收

```bash
probing $ENDPOINT eval "
import gc
import torch

# 强制垃圾回收
gc.collect()

# 清理 CUDA 缓存
if torch.cuda.is_available():
    torch.cuda.empty_cache()
    print('CUDA 缓存已清理')

print(f'垃圾回收完成: {gc.get_count()}')
"
```

## GPU 内存分析

### 当前 GPU 状态

```bash
probing $ENDPOINT eval "
import torch

if torch.cuda.is_available():
    for i in range(torch.cuda.device_count()):
        props = torch.cuda.get_device_properties(i)
        allocated = torch.cuda.memory_allocated(i) / 1024**3
        reserved = torch.cuda.memory_reserved(i) / 1024**3
        total = props.total_memory / 1024**3

        print(f'GPU {i}: {props.name}')
        print(f'  总计: {total:.2f} GB')
        print(f'  已分配: {allocated:.2f} GB')
        print(f'  已保留: {reserved:.2f} GB')
        print(f'  空闲: {total - reserved:.2f} GB')
"
```

## 最佳实践

### 1. 定期内存快照

在训练过程中定期拍摄快照：

```bash
probing $ENDPOINT eval "
import torch
step = trainer.current_step
if step % 100 == 0:
    allocated = torch.cuda.memory_allocated() / 1024**3
    print(f'步骤 {step}: {allocated:.2f} GB')
"
```

### 2. 分析内存密集型操作

```sql
-- 查找内存波动最大的操作
SELECT
  module,
  stage,
  stddev(allocated) as memory_variance,
  max(allocated) - min(allocated) as memory_range
FROM python.torch_trace
GROUP BY module, stage
HAVING stddev(allocated) > 10
ORDER BY memory_variance DESC;
```

## 故障排除

### 内存不足 (OOM)

1. 检查当前内存状态
2. 识别内存密集型模块
3. 强制垃圾回收
4. 减小批次大小或模型大小

### 内存未释放

1. 检查循环引用
2. 确认张量未被保存在列表/字典中
3. 显式使用 `del` 删除大张量
4. 调用 `torch.cuda.empty_cache()`

## 下一步

- [SQL 分析](sql-analytics.zh.md) - 更多查询模式
- [调试指南](debugging.zh.md) - 堆栈分析技术
- [常见问题](troubleshooting.zh.md) - 常见问题解决

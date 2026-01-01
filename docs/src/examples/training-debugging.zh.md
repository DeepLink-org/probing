# 训练调试示例

常见的训练调试场景和解决方案。

## 训练卡住

### 症状

- 损失停止更新
- 没有错误消息
- 进程仍在运行

### 诊断

```bash
# 1. 捕获堆栈跟踪
probing $ENDPOINT backtrace

# 2. 检查执行卡在哪里
probing $ENDPOINT query "
SELECT func, file, lineno, depth
FROM python.backtrace
ORDER BY depth
LIMIT 10"

# 3. 检查线程状态
probing $ENDPOINT eval "
import threading
for t in threading.enumerate():
    print(f'{t.name}: alive={t.is_alive()}')"
```

### 常见原因

#### 数据加载器阻塞

```bash
# 检查数据加载器
probing $ENDPOINT eval "
import torch.utils.data
print(f'DataLoader workers: {train_loader.num_workers}')
print(f'Prefetch factor: {train_loader.prefetch_factor}')"
```

#### 分布式死锁

```bash
# 检查分布式状态
probing $ENDPOINT eval "
import torch.distributed as dist
if dist.is_initialized():
    print(f'Rank: {dist.get_rank()}')
    print(f'World Size: {dist.get_world_size()}')"
```

## 损失爆炸

### 症状

- 损失变成 NaN 或 Inf
- 训练发散

### 诊断

```bash
# 检查梯度中的 NaN
probing $ENDPOINT eval "
import torch
for name, param in model.named_parameters():
    if param.grad is not None:
        has_nan = torch.isnan(param.grad).any().item()
        has_inf = torch.isinf(param.grad).any().item()
        if has_nan or has_inf:
            print(f'{name}: NaN={has_nan}, Inf={has_inf}')"
```

### 修复：梯度裁剪

```bash
# 检查当前裁剪设置
probing $ENDPOINT eval "
print(f'Grad clip value: {getattr(trainer, \"grad_clip\", None)}')"

# 应用紧急裁剪
probing $ENDPOINT eval "
torch.nn.utils.clip_grad_norm_(model.parameters(), 1.0)"
```

## 训练慢

### 诊断

```bash
# 找到最慢的模块
probing $ENDPOINT query "
SELECT
    module,
    stage,
    COUNT(*) as count,
    AVG(duration) as avg_time,
    SUM(duration) as total_time
FROM python.torch_trace
WHERE step > (SELECT MAX(step) - 5 FROM python.torch_trace)
GROUP BY module, stage
ORDER BY total_time DESC
LIMIT 10"
```

### 检查 GPU 利用率

```bash
probing $ENDPOINT eval "
import subprocess
result = subprocess.run(['nvidia-smi', '--query-gpu=utilization.gpu', '--format=csv,noheader'],
                        capture_output=True, text=True)
print(f'GPU 利用率: {result.stdout.strip()}')"
```

## 训练期间的内存问题

### 追踪内存增长

```bash
probing $ENDPOINT query "
SELECT
    step,
    MAX(allocated) as peak_memory,
    MAX(allocated) - MIN(allocated) as memory_range
FROM python.torch_trace
WHERE step > (SELECT MAX(step) - 20 FROM python.torch_trace)
GROUP BY step
ORDER BY step"
```

### 强制清理

```bash
probing $ENDPOINT eval "
import gc
import torch

# 清除梯度
model.zero_grad(set_to_none=True)

# 垃圾回收
gc.collect()

# 清理 CUDA 缓存
if torch.cuda.is_available():
    torch.cuda.empty_cache()

print('清理完成')"
```

## 检查点恢复

### 保存紧急检查点

```bash
probing $ENDPOINT eval "
import torch
checkpoint = {
    'step': trainer.current_step,
    'model': model.state_dict(),
    'optimizer': optimizer.state_dict(),
}
torch.save(checkpoint, 'emergency_checkpoint.pt')
print('紧急检查点已保存')"
```

### 检查检查点

```bash
probing $ENDPOINT eval "
import torch
ckpt = torch.load('checkpoint.pt', map_location='cpu')
print(f'Keys: {ckpt.keys()}')
print(f'Step: {ckpt.get(\"step\", \"N/A\")}')"
```

## 学习率问题

### 检查当前学习率

```bash
probing $ENDPOINT eval "
for i, pg in enumerate(optimizer.param_groups):
    print(f'Group {i}: lr={pg[\"lr\"]}, weight_decay={pg.get(\"weight_decay\", 0)}')"
```

### 调整学习率

```bash
# 降低学习率
probing $ENDPOINT eval "
for pg in optimizer.param_groups:
    pg['lr'] *= 0.1
print(f'新学习率: {optimizer.param_groups[0][\"lr\"]}')"
```

## 分布式训练问题

### 检查所有 Rank

```bash
# 在每个节点上运行
probing -t node1:8080 eval "print(f'Rank 0 step: {trainer.current_step}')"
probing -t node2:8080 eval "print(f'Rank 1 step: {trainer.current_step}')"
```

### 验证同步

```bash
probing $ENDPOINT eval "
import torch.distributed as dist
if dist.is_initialized():
    tensor = torch.tensor([trainer.current_step], device='cuda')
    dist.all_reduce(tensor)
    print(f'所有 rank 步数之和: {tensor.item()}')"
```

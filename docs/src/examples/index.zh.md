# 示例

展示 Probing 能力的真实示例。

## 概览

这些示例展示了 AI/ML 工作流中常见的调试和分析场景。

| 示例 | 描述 |
|------|------|
| [训练调试](training-debugging.zh.md) | 调试训练问题 |
| [内存泄漏](memory-leak.zh.md) | 查找和修复内存泄漏 |
| [性能分析](performance-analysis.zh.md) | 识别瓶颈 |

## 快速示例

### 检查训练进度

```bash
probing $ENDPOINT eval "
from probing.tracing import step_snapshot
snap = step_snapshot()
print(f'local_step: {snap.local_step}, global_step: {snap.global_step}')
"
```

### 监控 GPU 内存

```bash
probing $ENDPOINT eval "
import torch
allocated = torch.cuda.memory_allocated() / 1024**3
reserved = torch.cuda.memory_reserved() / 1024**3
print(f'已分配: {allocated:.2f} GB')
print(f'已保留: {reserved:.2f} GB')"
```

### 查找慢操作

```bash
probing $ENDPOINT query "
SELECT module, AVG(duration) as avg_time
FROM python.torch_trace
WHERE step > (SELECT MAX(step) - 5 FROM python.torch_trace)
GROUP BY module
ORDER BY avg_time DESC
LIMIT 5"
```

### 检查线程状态

```bash
probing $ENDPOINT eval "
import threading
for t in threading.enumerate():
    print(f'{t.name}: alive={t.is_alive()}, daemon={t.daemon}')"
```

## 运行示例

示例需要已启用 Probing 的目标进程（启动时 `PROBING=1`，或 Linux 上 `probing inject`）。

**环境：** 仓库内脚本需 [开发环境](../contributing.zh.md#development-setup)（`make develop`）。ML 示例可能需额外依赖 — 见 [examples/README.md](https://github.com/DeepLink-org/probing/blob/main/examples/README.md)。

```bash
# 设置端点
export ENDPOINT=12345  # 进程 ID
# 或
export ENDPOINT=host:8080  # 远程地址

# 运行示例命令
probing $ENDPOINT eval "..."
```

## 贡献示例

有实用的调试模式？欢迎贡献！

1. Fork 仓库
2. 将您的示例添加到 `docs/src/examples/`
3. 提交 Pull Request

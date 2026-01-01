# 扩展机制

Probing 提供机制来扩展其功能，支持自定义数据源和指标。

## 概览

扩展系统允许：

- 自定义数据表
- 用户定义的指标
- 与外部工具集成
- 插件架构

## 自定义表

### Python API

使用 `@table` 装饰器创建自定义表：

```python
from probing import table

@table("my_metrics")
def get_metrics():
    """返回字典列表或 pandas DataFrame。"""
    return [
        {"name": "loss", "value": current_loss},
        {"name": "accuracy", "value": current_acc},
    ]
```

### 查询自定义表

```sql
SELECT * FROM python.my_metrics;
```

### 表 Schema

表根据返回数据动态类型化：

```python
@table("training_state")
def get_training_state():
    return {
        "step": trainer.current_step,      # int
        "loss": trainer.last_loss,         # float
        "lr": optimizer.param_groups[0]["lr"],  # float
        "epoch": trainer.current_epoch,    # int
    }
```

## 外部表集成

### Pandas DataFrames

```python
import pandas as pd
from probing import register_table

df = pd.DataFrame({
    "timestamp": timestamps,
    "metric": values
})

register_table("external_metrics", df)
```

### Arrow 表

```python
import pyarrow as pa
from probing import register_table

table = pa.table({
    "id": [1, 2, 3],
    "value": [10.0, 20.0, 30.0]
})

register_table("arrow_data", table)
```

## 自定义指标

### 定义指标

```python
from probing import metric

@metric("gpu_utilization")
def gpu_util():
    """返回当前 GPU 利用率。"""
    import pynvml
    pynvml.nvmlInit()
    handle = pynvml.nvmlDeviceGetHandleByIndex(0)
    util = pynvml.nvmlDeviceGetUtilizationRates(handle)
    return util.gpu
```

### 查询指标

```sql
SELECT * FROM python.metrics WHERE name = 'gpu_utilization';
```

## 钩子系统

### 模块钩子

```python
from probing import register_hook

@register_hook("torch.nn.Linear", "forward")
def linear_hook(module, input, output):
    """每次 Linear 前向传播时调用。"""
    record_custom_data({
        "module": str(module),
        "input_shape": list(input[0].shape),
        "output_shape": list(output.shape),
    })
```

### 函数钩子

```python
from probing import hook_function

@hook_function("torch.optim.Adam.step")
def optimizer_hook(optimizer):
    """每次优化器步骤时调用。"""
    record_custom_data({
        "lr": optimizer.param_groups[0]["lr"],
        "step_count": optimizer.state_dict()["state"][0]["step"],
    })
```

## 插件架构

### 创建插件

```python
# my_plugin.py
from probing import Plugin

class MyPlugin(Plugin):
    name = "my_plugin"

    def on_load(self):
        """插件加载时调用。"""
        self.register_table("plugin_data", self.get_data)

    def on_unload(self):
        """插件卸载时调用。"""
        pass

    def get_data(self):
        return [{"key": "value"}]
```

### 加载插件

```bash
# 环境变量
PROBING_PLUGINS=my_plugin python train.py

# 或编程方式
import probing
probing.load_plugin("my_plugin")
```

## 配置扩展

### 自定义配置选项

```python
from probing import config

# 注册自定义配置
config.register("my_plugin.sample_rate", default=0.1, type=float)

# 在插件中使用
rate = config.get("my_plugin.sample_rate")
```

### 查询配置

```sql
SELECT * FROM information_schema.df_settings
WHERE name LIKE 'my_plugin.%';
```

## 集成示例

### Weights & Biases

```python
from probing import table
import wandb

@table("wandb_metrics")
def get_wandb_metrics():
    run = wandb.run
    if run:
        return {
            "run_id": run.id,
            "step": run.step,
            "summary": dict(run.summary),
        }
    return {}
```

### TensorBoard

```python
from probing import table
from torch.utils.tensorboard import SummaryWriter

writer = SummaryWriter()

@table("tensorboard_scalars")
def get_tb_scalars():
    # 访问 TensorBoard 数据
    return logged_scalars
```

### Prometheus

```python
from probing import metric
from prometheus_client import Gauge

gpu_memory = Gauge("gpu_memory_bytes", "GPU 内存使用")

@metric("prometheus_gpu_memory")
def update_prometheus():
    mem = torch.cuda.memory_allocated()
    gpu_memory.set(mem)
    return mem
```

## 最佳实践

### 1. 轻量级数据收集

```python
# 好：只返回必要数据
@table("efficient")
def get_efficient():
    return {"step": step, "loss": loss}

# 避免：昂贵的操作
@table("expensive")
def get_expensive():
    return serialize_entire_model()  # 太重
```

### 2. 错误处理

```python
@table("safe_data")
def get_safe_data():
    try:
        return {"value": compute_value()}
    except Exception as e:
        return {"error": str(e)}
```

### 3. 缓存

```python
from functools import lru_cache

@table("cached_data")
@lru_cache(maxsize=1)
def get_cached_data():
    return expensive_computation()
```

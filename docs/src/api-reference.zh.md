# API 参考

Probing CLI 命令和 Python API 的完整参考。

## CLI 命令

### probing inject

将探针注入到运行中的进程。

```bash
probing -t <pid> inject
```

**选项：**

- `-t, --target <pid>` - 目标进程 ID（必需）

**平台：** 仅 Linux

---

### probing query

对收集的数据执行 SQL 查询。

```bash
probing -t <endpoint> query "<sql>"
```

**示例：**

```bash
# 查询 torch 追踪
probing -t 12345 query "SELECT * FROM python.torch_trace LIMIT 10"

# 聚合查询
probing -t host:8080 query "SELECT module, AVG(duration) FROM python.torch_trace GROUP BY module"
```

---

### probing eval

在目标进程中执行 Python 代码。

```bash
probing -t <endpoint> eval "<python_code>"
```

**示例：**

```bash
# 简单执行
probing -t 12345 eval "print('hello')"

# 多语句
probing -t 12345 eval "import torch; print(torch.cuda.is_available())"
```

---

### probing backtrace

捕获当前堆栈跟踪。

```bash
probing -t <endpoint> backtrace
```

**输出：** 包含函数名、文件和行号的堆栈帧。

---

### probing repl

启动交互式 Python REPL。

```bash
probing -t <endpoint> repl
```

**功能：**

- Tab 补全
- 多行输入
- 命令历史

---

### probing list

列出启用了 probing 的进程。

```bash
probing list
```

**输出：** 进程 ID 及其 probing 状态。

---

### probing config

查看或修改配置。

```bash
# 查看所有配置
probing -t <endpoint> config

# 查看特定键
probing -t <endpoint> config probing.sample_rate

# 设置值
probing -t <endpoint> config probing.sample_rate=0.1
```

## Python API

### probing.connect

连接到 probing 端点。

```python
from probing import connect

# 通过 PID 连接
probe = connect(pid=12345)

# 通过地址连接
probe = connect(address="host:8080")
```

---

### @probing.table

注册自定义数据表。

```python
from probing import table

@table("my_data")
def get_my_data():
    return [{"key": "value"}]
```

## SQL 表

### python.backtrace

堆栈跟踪信息。

| 列 | 类型 | 描述 |
|----|------|------|
| func | string | 函数名 |
| file | string | 源文件 |
| lineno | int | 行号 |
| depth | int | 堆栈深度 |
| frame_type | string | Python/Native |

---

### python.torch_trace

PyTorch 执行跟踪。

| 列 | 类型 | 描述 |
|----|------|------|
| step | int | 训练步骤 |
| seq | int | 序列号 |
| module | string | 模块名 |
| stage | string | 钩子标签：`pre forward`、`post forward`、`pre step`、`post step`（时长在 post 行；默认不采 backward） |
| allocated | float | GPU 内存 (MB) |
| max_allocated | float | GPU 峰值内存 (MB) |
| cached | float | GPU 预留内存 (MB) |
| max_cached | float | 峰值预留 (MB) |
| time_offset | float | 相对 step 时间锚点（秒） |
| duration | float | 执行时间 (秒)；post 行有效 |

## 配置选项

| 键 | 默认值 | 描述 |
|----|--------|------|
| `probing.torch.profiling` | — | TorchProbe 配置（`on`、`ordered:0.5`、`random:0.1` 等） |
| `probing.pprof.sample_freq` | — | CPU pprof 采样频率 (Hz) |

## 环境变量

| 变量 | 描述 |
|------|------|
| `PROBING` | 启用 probing (1=开启) |
| `PROBING_PORT` | TCP 服务器端口 |
| `PROBING_TORCH_PROFILING` | TorchProbe（`on`、`ordered:0.5`、`random:0.1`、`tracepy=on` 等） |
| `PROBING_PPROF_SAMPLE_FREQ` | 同步为 `probing.pprof.sample_freq`（CPU pprof Hz） |
| `PROBING_AUTH_TOKEN` | 认证令牌 |

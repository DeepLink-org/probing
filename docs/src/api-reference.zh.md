# API 参考

CLI 命令与进程内 Python API。表结构见 **[SQL 表目录](reference/sql-tables.zh.md)**。

## CLI 命令

除特别说明外，命令均接受 `-t, --target <endpoint>`（`pid` 或 `host:port`）。

### 核心交互

| 命令 | 别名 | 说明 |
|------|------|------|
| `query "<sql>"` | `q` | 对 memtable 执行 SQL |
| `eval "<code>"` | `e` | 在目标进程执行 Python |
| `backtrace` | `bt`, `b` | 抓栈 → `python.backtrace` |
| `repl` | `r` | 交互式 Python REPL |

```bash
probing -t $ENDPOINT query "SELECT * FROM python.torch_trace LIMIT 10"
probing -t $ENDPOINT eval "import torch; print(torch.cuda.is_available())"
probing -t $ENDPOINT backtrace
```

### 发现与内省

| 命令 | 别名 | 说明 |
|------|------|------|
| `tables` | `tbl` | 列出可查询表（`--all` 含 `information_schema`） |
| `list` | `ls`, `l` | 列出已附着探针的进程 |
| `memory` | `mem` | 主机 RSS + GPU 内存采样 |
| `config [key[=value]]` | `cfg`, `c` | 查看或设置运行时配置 |
| `flamegraph [pprof\|torch]` | `flame`, `fg` | CPU pprof 或 Torch 模块火焰图 |
| `rdma [hca]` | `rd` | RDMA 流分析（若可用） |

```bash
probing -t $ENDPOINT tables
probing -t $ENDPOINT config probing.torch.profiling
probing -t $ENDPOINT config probing.torch.profiling=0.1
probing -t $ENDPOINT flamegraph torch -o torch.html
```

### 集群（分布式）

| 子命令 | 说明 |
|--------|------|
| `cluster nodes` | 列出已注册节点（`rank`、`role`、`status` 等） |
| `cluster query "<sql>"` | 扇出 SQL；结果含联邦标签（`_rank`、`_role` 等） |
| `cluster query --local "<sql>"` | 仅查询当前连接的 endpoint |

```bash
probing -t rank0:8080 cluster nodes
probing -t rank0:8080 cluster query "SELECT _rank, _role, AVG(duration) FROM global.python.comm_collective GROUP BY 1,2"
```

进程内等价：`probing.query("SELECT … FROM global.python.torch_trace …")`（需已注册 peer）。

### 诊断 skill

多步 SQL 剧本（CLI、Web Agent、MCP 共用）。来源包括内置 `skills/` 与已安装包注册的
`probing.skills` entry point。

| 子命令 | 说明 |
|--------|------|
| `skill list` | 列出已发现 skill（内置 + 厂商扩展） |
| `skill run <id>` | 对目标执行（`-p key=value`、`--global`、`--local`） |
| `skill install` | 安装到 Cursor / Claude / Codex skill 目录 |
| `skill update` | 从 bundle 更新已安装 skill |

```bash
probing -t $ENDPOINT skill list
probing -t $ENDPOINT skill run health_overview
probing -t $ENDPOINT skill run slow_rank --global
python -m probing.skills validate          # 开发：校验 skills/ 源码树
python -m probing.extensions skill-roots
python -m probing.extensions extensions
```

厂商扩展：`pip install probing-nvidia` — 见 **[扩展 — 厂商包](design/extensibility.zh.md#path-4-vendor-extension-package-probing-vendor)**、`examples/probing-acme/`。

Python 辅助 API（仅发现 / 展开计划 — 执行走 Rust CLI 或 MCP）：

```python
from probing.skills.tools import list_skills, plan_skill_run
plan_skill_run("health_overview")  # 返回 CLI 命令与步骤 SQL 预览
```

见 **[诊断 Skill](guide/skills.zh.md)**。

### 进程管理

| 命令 | 平台 | 说明 |
|------|------|------|
| `inject` | Linux | 向运行中 PID 注入探针 |
| `launch [--recursive] <args…>` | 全平台 | 以 probing 启用状态启动 Python |

```bash
probing -t $PID inject          # Linux 附着
PROBING=1 python train.py       # macOS / Windows / 训练推荐路径
```

---

## Python API（进程内）

训练脚本在 `PROBING=1`（或 Linux `inject` 后）使用。**没有** `probing.connect()`；远程一律 CLI `-t <endpoint>`。

### probing.query

```python
import probing

df = probing.query("SELECT * FROM python.torch_trace LIMIT 10")
```

### probing.span / probing.event / probing.record_span / probing.step

用户面四个动词：

```python
import probing

with probing.span("forward", phase=probing.FORWARD):
    probing.event("batch.stats", attributes=[{"loss": 1.25}])

probing.record_span("all_reduce", duration_ns=1_000_000)

probing.attach_training_phases(model, optimizer)  # hook 驱动 forward/backward/optimizer

probing.step.micro_step              # 最细计数
probing.step()                       # micro_step +1
probing.step(42)                     # 设置 micro_step
probing.step(micro_batches=10)       # 梯度累积分组
probing.step.local_step              # micro_step // micro_batches
probing.step.global_step             # = local_step
```

### probing.tracing 原语（集成 / 插件）

| 类别 | 导入 | 用途 |
|------|------|------|
| Span | `span`, `event`, `record_span`, `current_span` | 插桩 |
| Step | `step`, `step_fields` | 训练坐标 |
| Phase | `FORWARD`, `BACKWARD`, `OPTIMIZER`, `phases`, `attach_training_phases` | 训练阶段 |
| Integrator | `phases.infer_from_stage()` | Torch stage → 训练 phase |
| Context | `span_attrs`, `row_fields`, `step_fields` | span 与表行上下文字段 |
| Backend | `register_backend`, `configure_backends`, `list_backends`, `reset_backends` | 导出插件；内置：`memtable`、`logger`、`otel` |
| Table | `TraceEvent`, `SPANS_SQL` | SQL / skill |

```python
from probing.tracing import register_backend, configure_backends

with probing.span("forward", phase=probing.FORWARD, source="my_trainer"):
    ...

register_backend("my_sink", factory)
configure_backends(["memtable", "logger"])  # 终端 + memtable
configure_backends(["memtable", "my_sink"])
```

### @table（dataclass 插件）

```python
from dataclasses import dataclass
from probing import table

@table
@dataclass
class MyMetrics:
    step: int
    loss: float

def init():
    MyMetrics.init_table()

MyMetrics(step=1, loss=0.42).save()
```

见 **[扩展机制](design/extensibility.zh.md)**。

### probing.set_role / current_role / clear_role

```python
probing.set_role("dp=2,pp=1,tp=0")
probing.set_role(dp=2, pp=1, tp=0)
probing.current_role()
probing.clear_role()
```

### probing.tracing.step_snapshot

已合并为 ``probing.step()`` — 见上文 **probing.step**。

---

## 配置

| 键 | 说明 |
|----|------|
| `probing.torch.profiling` | TorchProbe（`on`、`0.5`、`0.1:0.3`、`tracepy=on` 等） |
| `probing.pprof.sample_freq` | CPU pprof 采样频率 (Hz) |

```bash
probing -t $ENDPOINT config
probing -t $ENDPOINT config probing.torch.profiling=0.1
```

**没有** `probing.sample_rate` 配置项。Torch 采样通过 `probing.torch.profiling` 或 `PROBING_TORCH_PROFILING` 控制。

### 环境变量

| 变量 | 说明 |
|------|------|
| `PROBING` | 启用 probing（`1`） |
| `PROBING_PORT` | 远程 CLI 的 TCP 端口 |
| `PROBING_TORCH_PROFILING` | TorchProbe 规格 |
| `PROBING_PPROF_SAMPLE_FREQ` | CPU pprof Hz |
| `PROBING_AUTH_TOKEN` | HTTP 认证令牌 |
| `PROBING_ROLE_<NAME>` | 自定义并行维度，参与 `role` 推导 |

---

## 文档提及但未实现 {#unimplemented-apis}

新代码请勿使用——便于迁移对照：

| API / 模式 | 请改用 |
|------------|--------|
| `probing.connect()` | CLI `probing -t <endpoint> …` |
| `@metric` 装饰器 | `@table` dataclass + `.save()` |
| 函数式 `@table` | 仅 dataclass + `@table` |
| `probing.sample_rate` 配置 | `probing.torch.profiling` / `PROBING_TORCH_PROFILING` |
| `probing.is_profiling_active()` | `probing tables` / `SELECT COUNT(*) FROM python.torch_trace` |
| `probing.flush()` | 事件触发即写入，无 flush API |
| `probing.get_config()`（Python） | CLI `probing config` |
| `cluster list` | `cluster nodes` |
| `nccl_trace` / `training_metrics` 表 | `python.comm_collective`、`nccl.proxy_ops`、`python.torch_trace` |

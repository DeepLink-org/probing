# API Reference

CLI commands and in-process Python API. Table schemas live in **[SQL Tables](reference/sql-tables.md)**.

## CLI commands

All commands accept `-t, --target <endpoint>` (`pid` or `host:port`) unless noted.

### Core interaction

| Command | Aliases | Description |
|---------|---------|-------------|
| `query "<sql>"` | `q` | Run SQL against memtables |
| `eval "<code>"` | `e` | Execute Python in the target process |
| `backtrace` | `bt`, `b` | Capture stack → `python.backtrace` |
| `repl` | `r` | Interactive Python REPL |

```bash
probing -t $ENDPOINT query "SELECT * FROM python.torch_trace LIMIT 10"
probing -t $ENDPOINT eval "import torch; print(torch.cuda.is_available())"
probing -t $ENDPOINT backtrace
```

### Discovery & introspection

| Command | Aliases | Description |
|---------|---------|-------------|
| `tables` | `tbl` | List queryable tables (`--all` includes `information_schema`) |
| `list` | `ls`, `l` | List processes with probes attached |
| `memory` | `mem` | Host RSS + GPU memory samples |
| `config [key[=value]]` | `cfg`, `c` | View or set runtime config |
| `flamegraph [pprof\|torch]` | `flame`, `fg` | CPU pprof or Torch module flamegraph |
| `rdma [hca]` | `rd` | RDMA flow analysis (when available) |

```bash
probing -t $ENDPOINT tables
probing -t $ENDPOINT config probing.torch.profiling
probing -t $ENDPOINT config probing.torch.profiling=ordered:0.1
probing -t $ENDPOINT flamegraph torch -o torch.html
```

### Cluster (distributed)

| Subcommand | Description |
|------------|-------------|
| `cluster nodes` | List registered cluster nodes (`rank`, `role`, `status`, …) |
| `cluster query "<sql>"` | Fan-out SQL; results include federation tags (`_rank`, `_role`, …) |
| `cluster query --local "<sql>"` | Query only the connected endpoint |

```bash
probing -t rank0:8080 cluster nodes
probing -t rank0:8080 cluster query "SELECT _rank, _role, AVG(duration) FROM global.python.comm_collective GROUP BY 1,2"
```

In-process equivalent: `probing.query("SELECT … FROM global.python.torch_trace …")` when peers are registered.

### Diagnostic skills

Structured multi-step SQL playbooks (shared with Web Agent):

| Subcommand | Description |
|------------|-------------|
| `skill list` | List bundled skills (`health_overview`, `slow_rank`, …) |
| `skill run <id>` | Run skill against target (`-p key=value`, `--global`, `--local`) |
| `skill install` | Copy skills into Cursor / Claude / Codex agent dirs |
| `skill update` | Refresh installed skills from bundle |

```bash
probing -t $ENDPOINT skill list
probing -t $ENDPOINT skill run health_overview
probing -t $ENDPOINT skill run slow_rank --global
python -m probing.skills validate   # dev: validate skills/ authoring tree
```

See **[Diagnostic Skills](guide/skills.md)** for workflow.

### Process management

| Command | Platform | Description |
|---------|----------|-------------|
| `inject` | Linux | Attach probe to running PID |
| `launch [--recursive] <args…>` | All | Start Python with probing enabled |

```bash
probing -t $PID inject          # Linux attach
PROBING=1 python train.py       # macOS / Windows / preferred for training
```

---

## Python API (in-process)

Use when the training script runs with `PROBING=1` (or after Linux `inject`). **There is no** `probing.connect()` — remote access is always CLI `-t <endpoint>`.

### probing.query

```python
import probing

df = probing.query("SELECT * FROM python.torch_trace LIMIT 10")
```

### probing.span / probing.event

```python
with probing.span("forward", kind="nn.forward"):
    ...
probing.event("batch.stats", attributes=[{"loss": 1.25}])
```

### @table (dataclass plugins)

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

See **[Extensibility](design/extensibility.md)**.

### probing.set_role / current_role / clear_role

```python
probing.set_role("dp=2,pp=1,tp=0")
probing.set_role(dp=2, pp=1, tp=0)
probing.current_role()
probing.clear_role()
```

### probing.tracing.step_snapshot

```python
from probing.tracing import step_snapshot

snap = step_snapshot()
# snap.local_step, snap.global_step — use in SQL filters and custom tables
```

---

## Configuration

| Key | Description |
|-----|-------------|
| `probing.torch.profiling` | TorchProbe (`on`, `ordered:0.5`, `random:0.1`, `tracepy=on`, …) |
| `probing.pprof.sample_freq` | CPU pprof sampling frequency (Hz) |

```bash
probing -t $ENDPOINT config
probing -t $ENDPOINT config probing.torch.profiling=ordered:0.1
```

There is **no** `probing.sample_rate` key. Torch sampling is controlled via `probing.torch.profiling` or `PROBING_TORCH_PROFILING`.

### Environment variables

| Variable | Description |
|----------|-------------|
| `PROBING` | Enable probing (`1`) |
| `PROBING_PORT` | TCP listen port for remote CLI |
| `PROBING_TORCH_PROFILING` | TorchProbe spec (mirrors `probing.torch.profiling`) |
| `PROBING_PPROF_SAMPLE_FREQ` | CPU pprof Hz |
| `PROBING_AUTH_TOKEN` | HTTP auth token |
| `PROBING_ROLE_<NAME>` | Custom parallel dimension for `role` derivation |

---

## Documented but not implemented {#unimplemented-apis}

Do not use these in new code — listed for migration clarity:

| API / pattern | Use instead |
|---------------|-------------|
| `probing.connect()` | CLI `probing -t <endpoint> …` |
| `@metric` decorator | `@table` dataclass + `.save()` |
| Function-style `@table` | Dataclass + `@table` only |
| `probing.sample_rate` config | `probing.torch.profiling` / `PROBING_TORCH_PROFILING` |
| `probing.is_profiling_active()` | `probing tables` / `SELECT COUNT(*) FROM python.torch_trace` |
| `probing.flush()` | Rows append on event; no flush API |
| `probing.get_config()` (Python) | CLI `probing config` |
| `cluster list` | `cluster nodes` |
| `nccl_trace` / `training_metrics` tables | `python.comm_collective`, `nccl.proxy_ops`, `python.torch_trace` |

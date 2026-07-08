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

Structured multi-step SQL playbooks (CLI, Web Agent, MCP). Skills from the bundled
tree and from installed packages that register `probing.skills` entry points.

| Subcommand | Description |
|------------|-------------|
| `skill list` | List discovered skills (`health_overview`, vendor extensions, …) |
| `skill run <id>` | Run skill against target (`-p key=value`, `--global`, `--local`) |
| `skill install` | Copy skills into Cursor / Claude / Codex agent dirs |
| `skill update` | Refresh installed skills from bundle |

```bash
probing -t $ENDPOINT skill list
probing -t $ENDPOINT skill run health_overview
probing -t $ENDPOINT skill run slow_rank --global
python -m probing.skills validate          # dev: validate skills/ authoring tree
python -m probing.extensions skill-roots
python -m probing.extensions extensions
```

Vendor packages: `pip install probing-nvidia` — see **[Extensibility — vendor package](design/extensibility.md#path-4-vendor-extension-package-probing-vendor)** and `examples/probing-acme/`.

Python helpers (discovery / plan only — execution is Rust CLI or MCP):

```python
from probing.skills.tools import list_skills, plan_skill_run
plan_skill_run("health_overview")  # CLI command + step SQL preview
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

### probing.span / probing.event / probing.record_span / probing.step

Four user-facing verbs:

```python
import probing

with probing.span("forward", phase=probing.FORWARD):
    probing.event("batch.stats", attributes=[{"loss": 1.25}])

probing.record_span("all_reduce", duration_ns=1_000_000)

probing.attach_training_phases(model, optimizer)  # hook-driven forward/backward/optimizer

probing.step.micro_step              # finest counter
probing.step()                       # micro_step +1
probing.step(42)                     # set micro_step
probing.step(micro_batches=10)       # gradient-accumulation grouping
probing.step.local_step              # micro_step // micro_batches
probing.step.global_step             # = local_step
```

### probing.tracing primitives (integrators / plugins)

| Category | Import | Purpose |
|----------|--------|---------|
| Span | `span`, `event`, `record_span`, `current_span` | Instrumentation |
| Step | `step`, `step_fields` | Training coordinates |
| Phase | `FORWARD`, `BACKWARD`, `OPTIMIZER`, `phases`, `attach_training_phases` | Training phase |
| Integrator | `phases.infer_from_stage()` | Torch stage → training phase |
| Context | `span_attrs`, `row_fields`, `step_fields` | Span and table row context fields |
| Backend | `register_backend`, `configure_backends`, `list_backends`, `reset_backends` | Export plugins; built-in: `memtable`, `logger`, `otel` |
| Table | `TraceEvent`, `SPANS_SQL` | SQL / skills |

```python
from probing.tracing import register_backend, configure_backends

with probing.span("forward", phase=probing.FORWARD, source="my_trainer"):
    ...

register_backend("my_sink", factory)
configure_backends(["memtable", "logger"])  # terminal + memtable
configure_backends(["memtable", "my_sink"])
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

### probing.step

Use ``probing.step()`` instead of the removed ``step_snapshot`` / ``sync_local_step`` helpers.
See **probing.span / probing.event / probing.record_span / probing.step** above.

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

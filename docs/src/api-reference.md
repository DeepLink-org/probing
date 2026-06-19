# API Reference

Complete reference for Probing's CLI commands and Python API.

## CLI Commands

### probing inject

Inject probes into a running process.

```bash
probing -t <pid> inject
```

**Options:**

- `-t, --target <pid>` - Target process ID (required)

**Platform:** Linux only

---

### probing query

Execute SQL queries against collected data.

```bash
probing -t <endpoint> query "<sql>"
```

**Examples:**

```bash
# Query torch traces
probing -t 12345 query "SELECT * FROM python.torch_trace LIMIT 10"

# Aggregate query
probing -t host:8080 query "SELECT module, AVG(duration) FROM python.torch_trace GROUP BY module"
```

---

### probing eval

Execute Python code in target process.

```bash
probing -t <endpoint> eval "<python_code>"
```

**Examples:**

```bash
# Simple evaluation
probing -t 12345 eval "print('hello')"

# Multi-statement
probing -t 12345 eval "import torch; print(torch.cuda.is_available())"
```

---

### probing backtrace

Capture current stack trace.

```bash
probing -t <endpoint> backtrace
```

**Output:** Stack frames with function names, files, and line numbers.

---

### probing repl

Start interactive Python REPL.

```bash
probing -t <endpoint> repl
```

**Features:**

- Tab completion
- Multi-line input
- Command history

---

### probing list

List processes with probing enabled.

```bash
probing list
```

**Output:** Process IDs and their probing status.

---

### probing config

View or modify configuration.

```bash
# View all config
probing -t <endpoint> config

# View specific key
probing -t <endpoint> config probing.sample_rate

# Set value
probing -t <endpoint> config probing.sample_rate=0.1
```

---

### probing memory

Quick memory overview.

```bash
probing -t <endpoint> memory
```

---

### probing rdma

RDMA flow analysis.

```bash
probing -t <endpoint> rdma
```

## Python API

### probing.connect

Connect to a probing endpoint.

```python
from probing import connect

# Connect by PID
probe = connect(pid=12345)

# Connect by address
probe = connect(address="host:8080")
```

---

### probe.eval

Execute code in target process.

```python
result = probe.eval("print('hello')")
```

---

### probe.query

Execute SQL query.

```python
df = probe.query("SELECT * FROM python.torch_trace")
```

---

### @probing.table

Register custom data table.

```python
from probing import table

@table("my_data")
def get_my_data():
    return [{"key": "value"}]
```

---

### @probing.metric

Register custom metric.

```python
from probing import metric

@metric("custom_metric")
def get_metric():
    return 42.0
```

## SQL Tables

### python.backtrace

Stack trace information.

| Column | Type | Description |
|--------|------|-------------|
| func | string | Function name |
| file | string | Source file |
| lineno | int | Line number |
| depth | int | Stack depth |
| frame_type | string | Python/Native |

---

### python.torch_trace

PyTorch execution traces.

| Column | Type | Description |
|--------|------|-------------|
| step | int | Training step |
| seq | int | Sequence number |
| module | string | Module name |
| stage | string | Hook label: `pre forward`, `post forward`, `pre step`, `post step` (duration on post rows; backward not collected by default) |
| allocated | float | GPU memory (MB) |
| max_allocated | float | Peak GPU memory (MB) |
| cached | float | Cached memory (MB) |
| max_cached | float | Peak cached memory (MB) |
| time_offset | float | Seconds since step time anchor |
| duration | float | Execution time (sec); meaningful on post rows |

---

### python.variables

Variable tracking.

| Column | Type | Description |
|--------|------|-------------|
| step | int | Training step |
| func | string | Function name |
| name | string | Variable name |
| value | string | String representation |

---

### information_schema.df_settings

Configuration settings.

| Column | Type | Description |
|--------|------|-------------|
| name | string | Setting name |
| value | string | Setting value |

## Configuration Options

| Key | Default | Description |
|-----|---------|-------------|
| `probing.torch.profiling` | — | TorchProbe spec (`on`, `ordered:0.5`, `random:0.1`, options) |
| `probing.pprof.sample_freq` | — | CPU pprof sampling frequency (Hz) |

## Environment Variables

| Variable | Description |
|----------|-------------|
| `PROBING` | Enable probing (1=on) |
| `PROBING_PORT` | TCP server port |
| `PROBING_TORCH_PROFILING` | TorchProbe (`on`, `ordered:0.5`, `random:0.1`, `tracepy=on`, …) |
| `PROBING_PPROF_SAMPLE_FREQ` | Synced to `probing.pprof.sample_freq` (CPU pprof Hz) |
| `PROBING_AUTH_TOKEN` | Authentication token |

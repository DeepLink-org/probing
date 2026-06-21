# Environment Variables

Complete reference of every environment variable Probing reads. Variables are grouped
by subsystem.

## Activation

| Variable | Values | Default | Description |
|----------|--------|---------|-------------|
| `PROBING` | `0`, `1`/`followed`, `2`/`nested`, `regex:PATTERN`, `SCRIPT.py` | unset (disabled) | Controls whether probing activates. `1` activates the current process. `2` activates current + child processes. `regex:PATTERN` activates when the script basename matches. `SCRIPT.py` activates when the script basename equals the value exactly. |
| `PROBING_ORIGINAL` | (set automatically) | — | Backs up the original `PROBING` value before probing modifies it. Set by site_hook; don't set manually. |

**Child-process propagation:** In `nested` mode, the original `PROBING` value is propagated to children. In `regex:` mode, non-matching children inherit `PROBING=1` so they can be inspected but won't re-trigger site hooks.

Prefix syntax: `init:SCRIPT+<mode>` runs `exec(open(SCRIPT).read())` after activation.

## Data storage

| Variable | Default | Description |
|----------|---------|-------------|
| `PROBING_DATA_DIR` | Platform-specific | Root directory for mmap ring buffer files (MEMT tables). Each process creates a subdirectory named by its PID. |
| `PROBING_COLD` | unset | Set to `on` to enable hot-to-cold compaction of mmap tables. |
| `PROBING_COLD_TARGET_MB` | — | Target size per cold chunk after compaction. |
| `PROBING_COLD_MAX_TOTAL_MB` | — | Maximum total size of all cold storage files. |
| `PROBING_COLD_TTL_SECS` | — | Minimum age of a chunk before it's eligible for cold compaction. |
| `PROBING_COLD_POLL_MS` | — | Interval between compaction poll cycles. |
| `PROBING_COLD_MAX_AGE_SECS` | — | Maximum age of a chunk before forced compaction. |
| `PROBING_COLD_DIR` | — | Directory for cold storage files (defaults under `PROBING_DATA_DIR`). |

## Server & networking

| Variable | Default | Description |
|----------|---------|-------------|
| `PROBING_PORT` | unset | TCP port for the embedded HTTP server. Set to `RANDOM` for automatic port selection. Required for remote access. |
| `PROBING_SERVER_ADDR` | Inferred from port | Explicit bind address (e.g. `0.0.0.0:8080`). |
| `PROBING_SERVER_ADDRPATTERN` | unset | IP pattern filter for multi-homed hosts. Selects the first matching interface. |
| `PROBING_SERVER_WORKER_THREADS` | auto | Number of Tokio worker threads. |
| `PROBING_CTRL_ROOT` | `/tmp/probing/` | Directory for Unix domain sockets (local PID-based connections). |
| `PROBING_MAX_REQUEST_SIZE` | server default | Maximum HTTP request body size in bytes. |
| `PROBING_MAX_FILE_SIZE` | server default | Maximum file upload size in bytes. |
| `PROBING_ALLOWED_FILE_DIRS` | server default | Colon-separated list of directories allowed for file reads. |
| `PROBING_BASE_PATH` | unset | URL path prefix for reverse proxy deployments (e.g. `/probing`). |
| `PROBING_REMOTE_QUERY_TIMEOUT_SECS` | server default | Timeout for remote fan-out queries (federation). |
| `PROBING_ASSETS_ROOT` | built-in default | Path to the web UI static assets directory. |

## Authentication

| Variable | Default | Description |
|----------|---------|-------------|
| `PROBING_AUTH_TOKEN` | unset | Bearer token for HTTP authentication. Required for remote access when set. |
| `PROBING_AUTH_USERNAME` | unset | Username for Basic authentication. |
| `PROBING_AUTH_REALM` | unset | Authentication realm string for Basic auth. |

## Tracing & spans

| Variable | Default | Description |
|----------|---------|-------------|
| `PROBING_SPAN_BACKENDS` | `memtable` | Comma-separated list of span backends. Built-in: `memtable` (writes to `python.trace_event`), `logger` (writes to stderr), `otel` (OpenTelemetry export). Custom backends can be registered via `probing.span_backends` entry point. |
| `PROBING_SPAN_LOG_LEVEL` | `INFO` | Log level for the `logger` span backend. |
| `PROBING_SPAN_LOCATION` | unset | Enable automatic location capture via `inspect.stack()` for every span. Adds overhead; use sparingly. |

## Step coordinates

| Variable | Default | Description |
|----------|---------|-------------|
| `PROBING_MICRO_BATCHES` | `1` | Initial gradient accumulation factor. Controls `local_step = micro_step // micro_batches`. |
| `PROBING_STEP_BUCKET` | — | Step bucket size for grouped storage. |
| `PROBING_GLOBAL_STEP_BUCKET` | — | Global step bucket size (falls back to `PROBING_STEP_BUCKET`). |

## Parallel topology (role)

Set these to describe your training's parallelism configuration. Probing combines
them into a `role` string like `dp=2,pp=1,tp=0`.

| Variable | Description |
|----------|-------------|
| `PROBING_TP_RANK` / `PROBING_TP_SIZE` | Tensor parallelism rank and size. |
| `PROBING_PP_RANK` / `PROBING_PP_SIZE` | Pipeline parallelism rank and size. |
| `PROBING_DP_RANK` / `PROBING_DP_SIZE` | Data parallelism rank and size. |
| `PROBING_EP_RANK` | Expert parallelism rank. |
| `PROBING_CP_RANK` | Context parallelism rank. |
| `PROBING_ROLE_<NAME>` | Arbitrary named parallelism dimension (e.g. `PROBING_ROLE_SP=8`). |

Non-PROBING-prefixed aliases are also recognized for Megatron compatibility:
`TP_RANK`, `TP_SIZE`, `PP_RANK`, `PP_SIZE`, `DP_RANK`, `DP_SIZE`,
`TENSOR_MODEL_PARALLEL_RANK`, `PIPELINE_MODEL_PARALLEL_RANK`,
`DATA_PARALLEL_RANK`, and more.

## CPU sampling

| Variable | Default | Description |
|----------|---------|-------------|
| `PROBING_CPU` | enabled | Set to `0`, `off`, `false`, or `no` to disable CPU sampling. |
| `PROBING_CPU_SAMPLE_MS` | `1000` | Sampling interval in milliseconds. Set to `0` to disable. |
| `PROBING_CPU_THREAD_TOP_N` | `8` | Maximum number of threads to sample per process per interval. |

## GPU sampling

| Variable | Default | Description |
|----------|---------|-------------|
| `PROBING_GPU` | enabled | Set to `0`, `off`, `false`, or `no` to disable GPU sampling. |
| `PROBING_GPU_SAMPLE_MS` | — | GPU sampling interval in milliseconds. |
| `PROBING_GPU_BACKEND` | `auto` | GPU backend filter: `auto`, `cuda`, `rocm`, `metal`. |

## NCCL & HCCL

| Variable | Description |
|----------|-------------|
| `PROBING_NCCL_MOCK` | Enable mock NCCL proxy data for testing without GPUs. |
| `PROBING_NCCL_PROFILER` | Path to the NCCL profiler shared library. |
| `PROBING_HCCL_PROFAPI_REAL` | Path to the real HCCL profapi library (Ascend NPU). |
| `PROBING_HCCL_SHIM` | Path to the HCCL shim library. |
| `PROBING_HCCL_SHIM_LOG` | Enable HCCL shim debug logging. |

## RDMA

| Variable | Default | Description |
|----------|---------|-------------|
| `PROBING_RDMA_HCA_NAME` | — | HCA device name filter for RDMA counter sampling. |
| `PROBING_RDMA_SAMPLE_RATE` | — | RDMA counter sampling rate in seconds. |

## PyTorch integration

| Variable | Description |
|----------|-------------|
| `PROBING_TORCH_PROFILING` | Set to `on` to activate PyTorch module hooks and write `python.torch_trace`. Required for module timing and memory data. |
| `PROBING_TORCHRUN_CLUSTER` | Enable automatic cluster registration via torchrun. |
| `PROBING_TORCHRUN_STORE_TIMEOUT` | Timeout for torchrun distributed store operations. |

## Debugging & diagnostics

| Variable | Default | Description |
|----------|---------|-------------|
| `PROBING_LOGLEVEL` | `info` | Rust-side log level: `trace`, `debug`, `info`, `warn`, `error`. |
| `PROBING_CRASH_BACKTRACE` | enabled | Print a backtrace on fatal signals (SIGSEGV, SIGABRT, etc.). Set to `0` to disable. |
| `PROBING_RUST_BACKTRACE` | — | Rust error backtrace detail (similar to `RUST_BACKTRACE`). |
| `PROBING_SAFE_DEMO` | — | Safe demonstration mode that restricts dangerous operations. |

## Skill & tool paths

| Variable | Description |
|----------|-------------|
| `PROBING_PROJECT_SKILLS_DIR` | Per-project skill directory (overrides `$PWD/.probing/skills/`). |
| `PROBING_USER_SKILLS_DIR` | Per-user skill directory (overrides `$HOME/.probing/skills/`). |
| `PROBING_CODE_ROOT` | Root directory for embedded Python monitoring code. |
| `PROBING_CLI_MODE` | Set automatically by the CLI to prevent recursive engine initialization. |
| `PROBING_PYTHON` | Path to the Python interpreter used by the CLI. Set automatically. |

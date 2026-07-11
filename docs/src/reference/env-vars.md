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
| `PROBING_SPAN_BACKENDS` | `memtable` | Comma-separated span backends. Built-in: `memtable` (`python.trace_event`), `logger` (stderr), `otel` (OpenTelemetry), `none` (stack only, no persistence). `configure_backends([])` also disables until `reset_backends()`. Unknown names fall back to `memtable` only. Custom backends: `probing.span_backends` entry point. See [Span API](../design/tracing-spans.md). |
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
| `PROBING_NCCL_MIN_MSG_BYTES` | Skip recording NCCL ops smaller than this size in bytes (default `0` = record all). |
| `PROBING_NCCL_INFLIGHT_THRESHOLD_SECS` | Watchdog threshold for snapshotting in-flight (possibly hung) NCCL ops into `nccl.inflight_ops` (default `10`, `0` disables). |
| `PROBING_HCCL_PROFAPI_REAL` | Path to the real HCCL profapi library (Ascend NPU). |
| `PROBING_HCCL_SHIM` | Path to the HCCL shim library. |
| `PROBING_HCCL_SHIM_LOG` | Enable HCCL shim debug logging. |

## RDMA

| Variable | Default | Description |
|----------|---------|-------------|
| `PROBING_RDMA_HCA_NAME` | — | HCA device name filter for RDMA counter sampling. |
| `PROBING_RDMA_SAMPLE_RATE` | — | RDMA counter sampling rate in seconds. |

## PyTorch integration

| Variable | Default | Description |
|----------|---------|-------------|
| `PROBING_TORCH_PROFILING` | — | Set to `on` to activate PyTorch module hooks and write `python.torch_trace`. Default when enabled: **5% step sampling** (`rate=0.05`), full-snapshot (`layer_rate=1.0`), **shadow cadence 4:1** (`shadow=4:1` — one baseline step per four probed steps for in-run overhead in `python.torch_step_timing`). Spec is `rate[:layer_rate]` (`layer_rate` = per-layer hit probability on a sampled step); a leading `random:`/`ordered:` token is accepted for back-compat (always `random`). Override with e.g. `1.0`, `0.05:0.1`, `shadow=8:2`, or `shadow=off`. **Backward** timing (`backward=on`) times each module's backward via output/input grad hooks; off by default. |
| `PROBING_TORCHRUN_CLUSTER` | `1` | Enable automatic torchrun cluster registration. Set to `0` to disable. |
| `PROBING_TORCHRUN_STORE_TIMEOUT` | — | Timeout for torchrun distributed store operations. |

### Megatron autostart

Megatron integration is **best-effort** and enabled by default when Megatron env vars
or modules are detected. No training-script changes are required beyond `PROBING=2`.

| Variable | Default | Description |
|----------|---------|-------------|
| `PROBING_MEGATRON` | `auto` | `auto` = on when Megatron env/modules detected; `on`/`off` to force. |
| `PROBING_MEGATRON_STEP_SYNC` | `auto` | Sync `probing.step` with Megatron iteration via wrapped `train_step`. |
| `probing.megatron.enable` | — | Config override for Megatron autostart (`probing.config.set`). |
| `probing.megatron.step_sync` | — | Config override for iteration sync. |

Import hooks run when `megatron.core.parallel_state` and `megatron.training.training`
load: parallel ranks flow into `probing.set_role`, and `train_step` aligns step
coordinates for SQL JOINs.

| Variable | Default | Description |
|----------|---------|-------------|
| `PROBING_FR_ON_WATCHDOG` | `auto` | On NCCL watchdog exceptions, snapshot Flight Recorder into probing tables. |
| `probing.fr.on_watchdog` | — | Config override for watchdog Flight Recorder snapshot. |

### PyTorch Flight Recorder

Probing can snapshot PyTorch NCCL Flight Recorder data via
`/apis/pythonext/flight-recorder/snapshot` and writes it to
`python.torch_nccl_flight_record` / `python.torch_nccl_pg_status`.
These variables are read by PyTorch, not Probing, but should be set before
`torch.distributed.init_process_group`.

| Variable | Default | Description |
|----------|---------|-------------|
| `TORCH_NCCL_TRACE_BUFFER_SIZE` | PyTorch default | Set to a positive value (for example `2000`) to enable Flight Recorder ring-buffer collection. |
| `TORCH_NCCL_DUMP_ON_TIMEOUT` | `false` | Let PyTorch dump Flight Recorder files automatically on watchdog timeout. |
| `TORCH_FR_DUMP_TEMP_FILE` | PyTorch default | Prefix/path for PyTorch Flight Recorder dump files. |
| `TORCH_NCCL_TRACE_CPP_STACK` | `false` | Include C++ stack traces in Flight Recorder entries. |
| `TORCH_NCCL_ENABLE_TIMING` | `false` | Add CUDA timing events for collectives; may add overhead. |
| `TORCH_SYMBOLIZE_MODE` | PyTorch default | C++ stack symbolization mode (`dladdr`, `addr2line`, `fast`). |

## Cluster heartbeat (torchrun)

Hierarchical side-channel registration when `WORLD_SIZE > 1`. See [torchrun cluster heartbeat](../design/torchrun-cluster.md).

| Variable | Default | Description |
|----------|---------|-------------|
| `PROBING_CLUSTER_REPORT` | `1` | Periodic heartbeat worker. `0` = HTTP only, no periodic PUT. |
| `PROBING_CLUSTER_REPORT_INTERVAL_SEC` | `10` | Base heartbeat interval (seconds). |
| `PROBING_CLUSTER_REPORT_MAX_INTERVAL_SEC` | `120` | Backoff cap (clamped below stale TTL). |
| `PROBING_CLUSTER_REPORT_BACKOFF_FACTOR` | `2` | Multiplier per stable tick. |
| `PROBING_CLUSTER_REPORT_BACKOFF` | `1` | Set to `0` to disable exponential backoff when stable. |
| `PROBING_CLUSTER_STALE_SEC` | `25` | Mark node `dead` after this many seconds without heartbeat. Should exceed max interval. |
| `PROBING_CLUSTER_DISCOVER_TIMEOUT_SEC` | `2` | Timeout per master/local0 discovery attempt. |
| `PROBING_CLUSTER_REPORT_TIMEOUT_SEC` | `5` | HTTP PUT timeout for cluster report. |
| `PROBING_CLUSTER_PRESET` | — | Used by `examples/run_cluster_multinode.sh`: `demo`, `fast`, or `steady`. |
| `PROBING_CLUSTER_FANOUT_HIERARCHICAL` | `1` | Hierarchical cluster query fan-out (coordinator → local0 → leaves). `0` = flat fan-out to every peer. See [Hierarchical fan-out](../design/hierarchical-fanout.md). |
| `PROBING_REMOTE_QUERY_TIMEOUT_SECS` | `30` | Per-peer timeout for remote federated / cluster queries (seconds). |
| `PROBING_FANOUT_CONCURRENCY` | `128` | Max concurrent in-flight remote fan-out HTTP requests per query. |
| `PROBING_NCCL_CHUNK_BYTES` | `65536` | NCCL profiler mmap ring chunk size (bytes). |
| `PROBING_NCCL_NUM_CHUNKS` | `64` | NCCL profiler mmap ring chunk count (~4 MiB total per table at defaults). |
| `PROBING_NCCL_MAX_COLL_SLOTS` | `512` | Max in-flight collective/P2P event slots per rank. |
| `PROBING_NCCL_MAX_PROXY_OP_SLOTS` | `8192` | Max in-flight proxy-op slots per rank. |
| `PROBING_NCCL_MAX_PROXY_STEP_SLOTS` | `32768` | Max in-flight proxy-step slots per rank. |
| `PROBING_NCCL_MAX_KERNEL_CH_SLOTS` | `8192` | Max in-flight kernel-channel slots per rank. |
| `PROBING_NCCL_MAX_NET_SLOTS` | `4096` | Max in-flight net-plugin slots per rank. |
| `PROBING_NCCL_POOL_SHARDS` | `8` | Shard slot pools by comm hash (1–64); total slot limits are divided evenly across shards. |
| `PROBING_NCCL_MIN_MSG_BYTES` | `0` | Drop events below this message size (bytes); `0` = record all. |

## Debugging & diagnostics

| Variable | Default | Description |
|----------|---------|-------------|
| `PROBING_LOGLEVEL` | `info` | Rust-side log level: `trace`, `debug`, `info`, `warn`, `error`. |
| `PROBING_ENGINE_FAIL_FAST` | — | When set to `1`/`true`, exit the process if engine initialization fails (default: server stays up but `/ready` returns 503 and queries fail). |
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

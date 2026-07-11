# Examples

Runnable scripts under `examples/`. They are **not** installed with `pip install probing` or `make develop`.

## Dependencies

| Script | Extra packages | Notes |
|--------|----------------|-------|
| **`tracing.py`** | `torch` | **Tracing 入门**（hook 驱动 phase，~80 行） |
| `hooks.py`, `test_probing.py` | none (beyond probing) | Good smoke tests |
| `imagenet.py`, `imagenet_with_span.py` | `torch`, `torchvision` | Needs ImageNet data path |
| `ray_tracing_example.py` | `ray` | Optional Ray integration |
| `bench_profiler.py` | varies | See script header |
| **`torch_probe_overhead_smoke.py`** | none | TorchProbe shadow-step overhead smoke (no GPU) |
| **`bench_instrumentation.py`** | `torch` (optional) | One-shot report: span / phase hooks / TorchProbe overhead (`make bench`) |
| **`nccl_profiler_overhead.py`** | `torch` (CUDA) | NCCL AllReduce overhead vs probing profiler — see below |

Install PyTorch into your dev venv (tracing 示例只需 torch；ImageNet 脚本还需 torchvision)：

```bash
source .venv/bin/activate
uv pip install torch
# ImageNet 示例: uv pip install torch torchvision
```

## Running with probing

Use the project venv after `make develop` (see [Contributing](../docs/src/contributing.md)):

```bash
source .venv/bin/activate
PROBING=1 python examples/tracing.py          # tracing 入门（推荐）
PROBING=1 python examples/test_probing.py --depth 2
PROBING=1 python examples/bench_instrumentation.py --quick
make bench          # full report, one table per group as each finishes
make bench-quick    # faster smoke
```

On Linux you can also attach with `probing -t <pid> inject` instead of `PROBING=1` at startup.

### NCCL profiler overhead (Linux + CUDA)

Micro-benchmark (synthetic NCCL callbacks, no GPU):

```bash
cargo bench -p probing-nccl-profiler --bench callback_path
```

End-to-end AllReduce overhead (baseline vs `NCCL_PROFILER_PLUGIN`):

```bash
make nccl-profiler-lib
./examples/run_nccl_profiler_bench.sh
# tune: NPROC=8 BENCH_ITERS=500 MSG_BYTES=4194304 ./examples/run_nccl_profiler_bench.sh
```

Results land in `/tmp/probing_nccl_bench_<pid>/{baseline,profiled}.json`. Compare manually:

```bash
python examples/nccl_profiler_overhead.py --compare baseline.json profiled.json
```

## More documentation

- [Examples (MkDocs)](../docs/src/examples/index.md)
- [Quick Start](../docs/src/quickstart.md)
- [Vendor extension template](probing-acme/) — `probing-<vendor>` with entry-point skills + magics

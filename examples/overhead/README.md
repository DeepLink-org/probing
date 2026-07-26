# Overhead

Instrumentation / TorchProbe / NCCL profiler 开销测量。

## 入口

```bash
./examples/overhead/run_bench.sh              # 完整报告
./examples/overhead/run_bench.sh --quick
./examples/overhead/run_torch_probe_smoke.sh  # 无 GPU
make nccl-profiler-lib && ./examples/overhead/run_nccl_bench.sh   # Linux + CUDA
```

Makefile：`make bench` / `make bench-quick` → `bench_instrumentation.py`。

## 文件

| 文件 | 说明 |
|------|------|
| `bench_instrumentation.py` | span / phase / TorchProbe 墙钟 |
| `torch_probe_overhead_smoke.py` | TorchProbe 冒烟 |
| `nccl_profiler_overhead.py` | AllReduce E2E |
| `run_nccl_bench.sh` | baseline vs profiled |
| `bench_profiler.py` | 早期综合 profiler 基准 |

设计文档：`docs/src/design/overhead.md`。

# Examples

Runnable scripts under `examples/`. They are **not** installed with `pip install probing` or `make develop`.

Framework integration **contract tests** live under `tests/regression/ext/` (e.g. `test_megatron_contract.py`, `test_vllm_contract.py`) — run via `make test-python-regression`. Real-framework smoke tests: `test_megatron_integration.py`, `test_vllm_integration.py` (`pytest -m integration`).

## Dependencies

| Script | Extra packages | Notes |
|--------|----------------|-------|
| **`tracing.py`** | `torch` | **Tracing 入门**（hook 驱动 phase，~80 行） |
| **`megatron_mcore_train_loop.py`** | `megatron-core`, `torch` (CUDA) | **真实 Megatron-Core 训练循环** + Web UI（`run_megatron_soak.sh`） |
| **`vllm_offline_soak.py`** | `vllm` / `vllm-metal` | **真实 vLLM 离线推理** + Web UI（`run_vllm_soak.sh`） |
| `hooks.py`, `test_probing.py` | none (beyond probing) | Good smoke tests |
| `imagenet.py`, `imagenet_with_span.py` | `torch`, `torchvision` | Needs ImageNet data path |
| `ray_tracing_example.py` | `ray` | Optional Ray integration |
| `bench_profiler.py` | varies | See script header |
| **`torch_probe_overhead_smoke.py`** | none | TorchProbe shadow-step overhead smoke (no GPU) |
| **`bench_instrumentation.py`** | `torch` (optional) | One-shot report: span / phase hooks / TorchProbe overhead (`make bench`) |
| **`run_soak.sh`** | `torch`, `torchvision` | Long-running soak (synthetic ImageNet + probing); used by CI `soak.yml` |
| **`run_imagenet_ddp.sh`** | `torch`, `torchvision` | **默认 2-rank torchrun** DDP demo（cluster query / 分布式火焰图 / `global.python.profile_*`） |
| **`run_megatron_soak.sh`** | `megatron-core`, CUDA | Megatron-Core torchrun soak（默认 2 GPU / TP=2，`PROBING_PORT=18080`） |
| **`run_vllm_soak.sh`** | `vllm` / `vllm-metal` | vLLM 真实推理 soak（默认 10min，`PROBING_PORT=18081`） |
| **`soak_assert.py`** | none (beyond probing) | Post-soak SQL / overhead assertions |
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
./examples/run_megatron_soak.sh               # Megatron 长跑 + Web UI
./examples/run_vllm_soak.sh                   # vLLM 长跑 + Web UI
make bench          # full report, one table per group as each finishes
make bench-quick    # faster smoke
make soak-quick     # ~60s CPU soak smoke (needs torch + torchvision)
```

Long-running soak (CI weekly job ``soak.yml``)::

```bash
# ~10 min single-process CPU soak + assertions
./examples/run_soak.sh

# shorter local smoke
DURATION_SEC=60 ./examples/run_soak.sh

# 2-rank gloo DDP (assertions on rank 0)
NPROC=2 DIST_BACKEND=gloo DURATION_SEC=120 ./examples/run_soak.sh

# dedicated distributed demo (default 2 ranks, Web UI on :18080)
./examples/run_imagenet_ddp.sh
DURATION_SEC=60 ./examples/run_imagenet_ddp.sh
```

On Linux you can also attach with `probing -t <pid> inject` instead of `PROBING=1` at startup.

### Megatron autostart

Regression (mocked modules, CI): `tests/regression/ext/test_megatron_contract.py` — `make test-python-regression`.

**Real Megatron-Core** training loop + Web UI (**Linux + NVIDIA CUDA only**; requires `megatron-core`, `triton`, `torchrun`):

```bash
# Linux GPU machine only (not macOS)
uv pip install megatron-core torch triton   # CUDA wheels
./examples/run_megatron_soak.sh
DURATION_SEC=60 ./examples/run_megatron_soak.sh
NPROC=1 TP_SIZE=1 ./examples/run_megatron_soak.sh   # single GPU
# browser: http://127.0.0.1:18080/
```

Based on NVIDIA [run_simple_mcore_train_loop.py](https://github.com/NVIDIA/Megatron-LM/blob/main/examples/run_simple_mcore_train_loop.py). Probing hooks `megatron.core.parallel_state` for role sync and aligns `probing.step` each optimizer iteration.

Full Megatron-LM `pretrain_gpt.py` jobs: set `PROBING=2 PROBING_MEGATRON=on` at launch (uses `megatron.training.training.train_step` hooks).

### vLLM / vLLM-Metal autostart

Regression (mocked modules, CI): `tests/regression/ext/test_vllm_contract.py` — `make test-python-regression`.

**Real vLLM** offline inference + Web UI:

```bash
# Linux/CUDA
uv pip install vllm
./examples/run_vllm_soak.sh

# macOS Apple Silicon (vllm-metal)
# https://github.com/vllm-project/vllm-metal
#
# Align vllm / vllm-metal / mlx-lm versions per vllm-metal docs. The soak defaults to
# VLLM_PROMPTS_PER_BATCH=1 on macOS because multi-prompt batched decode needs
# mlx_lm.BatchKVCache.merge (not in all mlx-lm releases).
uv pip install torchvision torchaudio 'transformers>=4.46,<5'
./examples/run_vllm_soak.sh
# multi-prompt batching (when mlx-lm supports BatchKVCache.merge):
# VLLM_PROMPTS_PER_BATCH=4 ./examples/run_vllm_soak.sh
DURATION_SEC=60 ./examples/run_vllm_soak.sh
# browser: http://127.0.0.1:18081/
```

`vllm serve` API server: `PROBING=2 PROBING_PORT=18081 vllm serve <model>` (engine hooks apply on load).

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

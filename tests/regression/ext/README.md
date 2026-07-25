# Framework extension tests (`tests/regression/ext/`)

Contract + integration tests for Megatron-Core and vLLM / vLLM-Metal probing hooks.

## Layout

| Module | Layer | What it validates |
|--------|-------|-------------------|
| `_megatron_contract.py` | Helpers | Mock Megatron module tree (not collected) |
| `_vllm_contract.py` | Helpers | Mock vLLM module tree (not collected) |
| `test_megatron_contract.py` | Contract | Import hooks, role/step sync (mock, fast CI) |
| `test_vllm_contract.py` | Contract | Import hooks, role/step sync (mock, fast CI) |
| `test_megatron_integration.py` | Integration | Real `megatron.core` via `torchrun` subprocess (CUDA) |
| `test_megatron_real_lm.py` | Integration (opt-in) | Official Megatron-LM + bottom fakes (`PROBING_MEGATRON_REAL_LM=1`) |
| `test_vllm_integration.py` | Integration | Real `vllm` import hook + optional generate smoke |
| `megatron_integration_worker.py` | Worker | Subprocess entry for Megatron integration |
| `vllm_integration_worker.py` | Worker | Subprocess entry for vLLM integration |

Shared fixtures: `conftest.py` (resets ext state + `probing.step` between tests).

## Run

```bash
# CI / default regression (contract tests only need probing._core)
make test-python-regression

# Contract only (fast)
PROBING=1 pytest tests/regression/ext/test_vllm_contract.py \
                 tests/regression/ext/test_megatron_contract.py -q

# Integration (skipped when framework not installed)
PROBING=1 pytest -m integration tests/regression/ext/test_vllm_integration.py -q
PROBING=1 pytest -m integration tests/regression/ext/test_megatron_integration.py -q

# Real vLLM generate smoke (opt-in; aligns with examples/inference/run_vllm_soak.sh)
PROBING_VLLM_RUN_GENERATE=1 PROBING=1 pytest \
  tests/regression/ext/test_vllm_integration.py::test_vllm_offline_generate_smoke_subprocess -q

# Official Megatron-LM + bottom fakes (opt-in; default ../Megatron-LM or MEGATRON_LM=)
PROBING_MEGATRON_REAL_LM=1 PROBING=1 pytest -m integration \
  tests/regression/ext/test_megatron_real_lm.py -q
```

## Megatron-LM checkout (real-LM path)

| Item | Value |
|------|-------|
| Env | `MEGATRON_LM=/path/to/Megatron-LM` |
| Default | `<probing-repo>/../Megatron-LM` (sibling) |
| Multi-version | **One checkout at a time** — switch by changing `MEGATRON_LM`; no in-process matrix |
| Smoke range | megatron-core **≥0.12.1, <0.21** (`MEGATRON_LM_ALLOW_ANY_VERSION=1` to override) |

Unit tests for path/version: `tests/unit/probing/fakes/test_megatron_lm_env.py`.

## Notes

- **Contract tests** never import real `megatron.core` or `vllm`; they mock `sys.modules`.
- **Megatron integration** (`test_megatron_integration.py`) requires Linux + CUDA + `megatron-core`; skipped on macOS.
- **Megatron real-LM** (`test_megatron_real_lm.py`) is opt-in (`PROBING_MEGATRON_REAL_LM=1`); uses bottom fakes and can run on macOS/CPU.
- **vLLM generate integration** is opt-in (`PROBING_VLLM_RUN_GENERATE=1`) because it is slow and
  depends on aligned vllm / vllm-metal / mlx-lm versions. Import-hook wiring runs without that flag.

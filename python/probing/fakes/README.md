# probing.fakes — macOS / no-CUDA bottom-layer fakes

Two modes:

1. **Real Megatron-LM** (preferred) — run the checkout's `pretrain_gpt.py` with
   only bottom-layer replacements (CUDA→cpu, triton/flash_attn stubs, FakeStore).
2. **Scripted fakes** — invent a Megatron-shaped surface for role/step debugging
   without a Megatron checkout (`meta` device; no real forward).

## Real Megatron-LM (bottom fakes only)

Requires a sibling checkout (or `MEGATRON_LM=`):

```bash
# ../Megatron-LM next to probing
PROBING=1 python examples/megatron/run_megatron_lm_pretrain.py --train-iters 2
```

What is **kept real**: `megatron.core` / `megatron.training` / `pretrain_gpt.py`.

What is **faked at the bottom**:

| Layer | Mechanism |
|-------|-----------|
| CUDA device APIs | remap to `cpu` (default for this runner; `meta`/`mps` via `--device`) |
| `triton` / `flash_attn` | MetaPath stubs |
| Distributed | Megatron `--fake-process-group` |
| Dataset C++ helpers | pure-Python `helpers_cpp` fallback |
| `torch.compile` | noop |

Do **not** invent `megatron.*` or `transformer_engine` on this path — Megatron
falls back to `--transformer-impl local` + Torch Norm / AdamW.

## Scripted / import-surface fakes

```bash
PROBING=1 python examples/megatron/pretrain_gpt.py --train-iters 4
PROBING=1 python examples/megatron/megatron_meta_debug_loop.py
```

```python
from probing.fakes import install, run_scripted_loop

install(force=True)  # cuda → meta; fake megatron / te / apex / flash_attn
run_scripted_loop(steps=4, tp=1, dp=0)
```

## Environment

| Variable | Default | Meaning |
|----------|---------|---------|
| `PROBING_FAKES` | unset (off) | `1`/`all` or comma list of specs |
| `PROBING_FAKES_FORCE` | unset | `1` to shadow real packages |
| `PROBING_FAKE_DEVICE` | `meta` (scripted) / `cpu` (real Megatron runner) | Remap target for CUDA APIs |
| `MEGATRON_LM` | `../Megatron-LM` (sibling of probing repo) | Official Megatron-LM checkout |
| `MEGATRON_LM_ALLOW_ANY_VERSION` | unset | `1` to skip 0.12.1–0.20.x smoke gate |

### Megatron-LM location & versions

Resolution order: **`MEGATRON_LM` / `--megatron-lm` → sibling `../Megatron-LM`**.

- **Not multi-version in-process.** Point `MEGATRON_LM` at another tree to switch
  (e.g. `Megatron-LM-0.19` vs `Megatron-LM-main`). Pip `megatron-core` is purged
  from `sys.path` / `sys.modules` once the checkout is bootstrapped.
- Smoke CLI defaults target **megatron-core ≥0.12.1 and <0.21**. Outside that range,
  set `MEGATRON_LM_ALLOW_ANY_VERSION=1`.
- Unit coverage: `tests/unit/probing/fakes/test_megatron_lm_env.py`.
- Opt-in regression: `PROBING_MEGATRON_REAL_LM=1 pytest -m integration tests/regression/ext/test_megatron_real_lm.py`.

Built-in specs: `megatron`, `transformer_engine` (`te`), `apex`, `flash_attn`, `triton`.

## Correlation / verify

Fake steps and hooked `torch.distributed` collectives append `python.fake_event`
rows. Collectives also dual-write `python.comm_collective`.

```python
from probing.fakes import verify_against_probing

report = verify_against_probing(require_train_steps=4)
report.raise_if_failed()
```

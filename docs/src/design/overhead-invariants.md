# TorchProbe overhead invariants (agents / maintainers)

**Do not change semantics below without updating tests.** Read this before editing `web/src/overhead/`, `python/probing/profiling/torch_probe.py`, `python/probing/profiling/deferred_drain.py`, or `skills/health_overview/steps.yaml` overhead SQL.

Background: [overhead.md](overhead.md). 中文: [中文版](/zh/design/overhead-invariants/).

## Invariants (summary)

| ID | Rule |
|----|------|
| **I1** | Primary % use **median** ratios, not `mean(probed)/mean(shadow)` |
| **I2** | Amortized = `(1−rate)×dispatch + rate×sampled`, not mean amortization |
| **I3** | `_record_step_timing` before `_drain_deferred` in `_close_step_wall` |
| **I4** | Deferred drain async by default (`PROBING_TORCH_DEFER_ASYNC=1`) |
| **I5** | Stable gate: `shadow_n≥5`, `dispatch_n≥16`, `shadow_baseline>0` |
| **I6** | UI soft formatting: `≈0%`, `~2%` for &lt;5%; Typical vs Effective labels |

## Test map

```bash
cd web && cargo test overhead
PROBING=0 pytest tests/regression/profiling/ -q
```

| Tests | Guards |
|-------|--------|
| `web/src/overhead/metrics.rs` | I1, I2, I5, I6 |
| `tests/regression/profiling/test_overhead_invariants.py` | I3, I4 |
| `test_torch_probe_sampling.py` | I3, defer settle |
| `test_deferred_drain_worker.py` | I4 |

See the Chinese doc for formulas, checklist, and the user regression fixture.

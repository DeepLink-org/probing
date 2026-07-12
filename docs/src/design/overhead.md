# Overhead measurement and formulas

Canonical design doc for **instrumentation overhead** in Probing: terminology, formulas, measurement boundaries, and offline vs in-run methods. Implementation details: [Profiling](profiling.md); table columns: [SQL tables — torch_step_timing](../reference/sql-tables.md#python-torch_step_timing).

**Invariants and regression tests** (read before changing formulas or hook order): [overhead-invariants.md](overhead-invariants.md).

## 1. Goals and scope

### Questions answered

| Question | Typical consumer |
|----------|------------------|
| How much slower is each training step with TorchProbe hooks? | Production jobs, `health_overview`, Web Overhead panel |
| What does span / memtable persistence cost? | `make bench` tracing layer, regression tests |
| End-to-end delta on a real TinyNet loop? | `make bench` torch_train layer |
| NCCL profiler impact on collective latency? | `run_nccl_profiler_bench.sh` (offline) |

### Out of scope

- **Business throughput SLOs** (tokens/s, time-to-convergence) — requires your own baseline job
- **NCCL collective execution time** as “probe tax” — that is training work, not instrumentation
- **“Pure PyTorch” on shadow steps** — other collectors (NCCL, CPU/GPU) still run on shadow steps

### vs `probing bench` CLI

| Entry | Measures |
|-------|----------|
| `make bench` → `examples/bench_instrumentation.py` | Python/Rust **instrumentation** wall time |
| `probing bench write` (hidden CLI) | memtable **write-path** throughput/latency — not training hook tax |

These are not interchangeable.

---

## 2. Terminology

| Term | Definition |
|------|------------|
| **probed step** | `python.torch_step_timing.is_shadow = 0`; TorchProbe module hooks run per config |
| **shadow step** | `is_shadow = 1`; module/optimizer hooks **return immediately**; no `python.torch_trace`, but a timing row is written |
| **shadow cadence** | Default `4:1` — 4 probed steps then 1 shadow step (`shadow=4:1`) |
| **sampled step** | probed with `sampled = 1` — module trace flush (may include GPU defer/sync) |
| **hook tax** | Median overhead %: all probed steps vs shadow |
| **sampled overhead** | Median overhead %: sampled probed steps only vs shadow |
| **total overhead (amortized)** | Weighted: `(1−rate)×dispatch_overhead + rate×sampled_overhead` (Web UI) |
| **in-run** | Continuous comparison via `python.torch_step_timing` during training |
| **offline** | Standalone benchmark scripts with A/B or paired comparison |

---

## 3. Common formulas

### 3.1 Relative overhead (ratio)

Used for TorchProbe in-run and many offline cases:

$$
\text{overhead\_pct} = \left(\frac{M_{\text{probed}}}{M_{\text{shadow}}} - 1\right) \times 100
$$

$M$ is an aggregate (see §5). SQL uses `nullif(..., 0)` when the denominator is zero.

### 3.2 Relative baseline (offline A/B)

$$
\text{vs\_baseline\_pct} = \left(\frac{T_{\text{measured}}}{T_{\text{baseline}}} - 1\right) \times 100
$$

### 3.3 Paired training delta

$$
\Delta T = T_{\text{instrumented}} - T_{\text{baseline}}, \quad T_{\text{inst\_med}} = \text{median}(T_{\text{baseline}}) + \text{median}(\Delta T)
$$

### 3.4 NCCL latency delta

$$
\text{pct\_delta}(ref, x) = 100 \times \frac{x - ref}{ref}
$$

Positive means the profiled mode is slower.

---

## 4. In-run measurement: TorchProbe shadow step

### 4.1 Mechanism

With `PROBING_TORCH_PROFILING=on` (default includes `shadow=4:1`), each optimizer step writes one row to `python.torch_step_timing`.

**Shadow detection** (`shadow_step_in_cycle`):

```text
cycle_len = shadow_normal + shadow_baseline   # default 4 + 1 = 5
is_shadow = (cycle_index % cycle_len) >= shadow_normal
```

Indices 0–3 are probed; index 4 is shadow (default cadence).

### 4.2 Timing window (`step_duration_sec`)

Uses `time.perf_counter()` wall clock — **not** GPU kernel time.

**Boundaries**:

- **Start**: `_mark_step_wall_start()` at the end of the previous step’s `post_step_hook`
- **End**: `_record_step_timing()` inside the current step’s `post_step_hook` (**before** `_drain_deferred()`)

So `step_duration_sec` covers **full step compute (forward/backward/opt) plus in-step hook teardown** (including same-step trace flush on sampled steps). It **excludes** deferred GPU `elapsed_time` drain from earlier sampled steps (drain runs after timing, before the next `_mark_step_wall_start()`).

**vs `train.step` span**: from `python.trace_event`, measures the wrapped compute interval only — **excludes** hook dispatch and persistence. The Web UI shows both; do not compare numbers directly.

### 4.3 Hook behavior by step type

| Type | Module hooks | Writes torch_trace | Timing row |
|------|--------------|-------------------|------------|
| discovery (first step) | register modules | no | special path |
| probed, not sampled | short-circuit | no | `is_shadow=0, sampled=0` |
| probed, sampled | full path | yes | `is_shadow=0, sampled=1` |
| shadow | immediate return | no | `is_shadow=1` |

`hook_tax` blends sampled and non-sampled probed steps. **`dispatch_overhead`** (`sampled=0` only) is the stable primary metric. `sampled_overhead` is the heavy path.

---

## 5. Metric catalog

### 5.1 In-run metrics

| Metric | Numerator | Denominator | Aggregate | Window | Consumer |
|--------|-----------|-------------|-----------|--------|----------|
| **dispatch_overhead_pct** | probed & `sampled=0` | shadow | **median** | rolling **80** steps | **primary alert** |
| **hook_tax_pct** | all probed | shadow | **median** | rolling **80** steps | blended / soak |
| **sampled_overhead_pct** | probed & `sampled=1` | shadow | **median** | rolling **80** steps | Web |
| **Dispatch (Web)** | `sampled=0` probed | shadow | **median** | last **80** steps | Web sidebar / panel |
| **Total overhead (Web)** | dispatch + sampled weighted by sample rate | — | **weighted %** | last 80 steps | amortized display |

**Reference SQL** (rolling window + stratified metrics):

```sql
WITH bounds AS (
  SELECT GREATEST(COALESCE(MAX(local_step), 0) - 80, 1) AS win_start
  FROM python.torch_step_timing
)
SELECT
  round((median(CASE WHEN is_shadow = 0 AND sampled = 0 THEN step_duration_sec END)
        / nullif(median(CASE WHEN is_shadow = 1 THEN step_duration_sec END), 0) - 1) * 100, 2)
    AS dispatch_overhead_pct,
  round((median(CASE WHEN is_shadow = 0 THEN step_duration_sec END)
        / nullif(median(CASE WHEN is_shadow = 1 THEN step_duration_sec END), 0) - 1) * 100, 2)
    AS hook_tax_pct,
  sum(CASE WHEN is_shadow = 0 AND sampled = 0 THEN 1 ELSE 0 END) AS dispatch_n,
  sum(CASE WHEN is_shadow = 1 THEN 1 ELSE 0 END) AS shadow_n
FROM python.torch_step_timing, bounds
WHERE local_step >= bounds.win_start AND local_step > 1;
```

**Display** (Web): $|\text{pct}| < 0.5$ → `≈0%`; treat as stable when `shadow_n ≥ 5` and `dispatch_n ≥ 16` (§5.2, §11).

### 5.2 Sample size

| Condition | Behavior |
|-----------|----------|
| `shadow_baseline = 0` (`shadow=off`) | overhead % undefined |
| `shadow_n < 5` | low-sample hint; % is indicative only |
| `dispatch_n < 16` | high variance on dispatch overhead |
| `probed_n = 0` or `shadow_n = 0` | soak skips ratio assertion |

### 5.3 Auxiliary metrics

| Metric | Source | Purpose |
|--------|--------|---------|
| `train_step_median_ms` | `median(span_end - span_start)` for `train.step` | compute-only reference |
| `nccl.profiler_counters` | latest row | data health, not overhead % |

---

## 6. Offline benchmark: `make bench`

Script: `examples/bench_instrumentation.py`. Run inside a **probing-injected** process:

```bash
PROBING=1 make bench
PROBING=1 make bench-quick
```

### 6.1 Parameters

| Parameter | full | `--quick` |
|-----------|------|-----------|
| span_iters | 300 | 80 |
| probe_steps | 40 | 12 |
| train batches | 30 | 8 |
| warmup (outer rounds) | 2 | 1 |
| runs (outer rounds) | 5 | 3 |

### 6.2 Three layers

**A — tracing**: median total wall time per scenario; `vs_baseline_pct` vs `span (no backend)`.

**B — torch_probe (synthetic)**: fake single module; median per-step timing; `hook_tax_pct` on `shadow=4:1` uses §3.1. Not representative of large production models.

**C — torch_train (TinyNet)**: paired back-to-back baseline vs instrumented; reports `paired_delta_ms` and `inst_med`.

### 6.3 JSON export

```bash
PROBING=1 python examples/bench_instrumentation.py --json-out /tmp/bench.json
```

---

## 7. NCCL profiler overhead (offline only)

No in-run shadow. Runtime health: `nccl.profiler_counters`.

E2E: `examples/nccl_profiler_overhead.py`, `examples/run_nccl_profiler_bench.sh`. Baseline vs plugin + `PROBING=2`. Compare with §3.4 on latency and throughput.

Micro: `probing/extensions/nccl-profiler/benches/callback_path.rs` (Criterion) — component-level only.

---

## 8. Other subsystems

| Subsystem | Method | Threshold |
|-----------|--------|-----------|
| Span + memtable | `tests/regression/profiling/test_span_overhead.py` | `T_on < T_off × 8 + 0.05s` |
| TorchProbe module spans | same | `med_on < med_off × 6 + 0.02s` |
| memtable writes | `probing bench write` | throughput / latency |
| pprof | `probing.pprof.sample_freq` | qualitative |

Isolate stack cost: `PROBING_SPAN_BACKENDS=none`.

---

## 9. Gates (current repo)

| Gate | Condition | Location |
|------|-----------|----------|
| diagnostic warning | `dispatch_overhead_pct > 5%` | `health_overview` |
| soak failure | `hook_tax_pct > 75%` default | `soak_assert.py` |
| CI regression | span ratio bounds | `test_span_overhead.py` |

5%, 75%, and 8× serve different purposes — not a single SLO.

---

## 10. Known biases

1. Shadow is not vacuum — other collectors still run.
2. Sampled steps may include GPU defer/sync in wall time.
3. Web uses 80-step window; skills use full history.
4. Synthetic bench ≠ production.
5. Filter `local_step > 1` to skip discovery.

---

## 11. Anti-noise and stability

### 11.1 Noise sources

| Source | Mitigation |
|--------|------------|
| Sampled steps mixed into hook tax | Use **`dispatch_overhead`** (`sampled=0`) as primary |
| Step-time jitter (data, collectives) | **median** + 80-step rolling window |
| Few shadow points | Require `shadow_n ≥ 5`; prefer long runs |
| Cold start / discovery | `local_step > 1`; offline warmup |
| Blended aggregation | Report dispatch / sampled / blended separately |
| Deferred drain blocks hook | After timing: async worker (`PROBING_TORCH_DEFER_ASYNC=1`, default) + bounded queue; sync fallback when full |

### 11.2.1 Async deferred drain

Ready `DelayedRecord` items are enqueued after `_record_step_timing`; a daemon thread runs `elapsed_time` + `save()`. Set `PROBING_TORCH_DEFER_ASYNC=0` for synchronous drain (tests). Queue size: `PROBING_TORCH_DEFER_QUEUE_SIZE` (default 4096). `atexit` flushes the queue; tests may call `flush_deferred_drain()`.

### 11.2 Principles

1. **Stratify** light (dispatch) vs heavy (sampled) paths.
2. **Robust aggregates**: median for alerts; amortized uses sample-rate-weighted blend, not `mean(probed)/mean(shadow)`.
3. **Aligned window**: Web and `health_overview` use **80 steps**.
4. **Paired offline bench** for torch_train layer.
5. **Explicit gates**: no alert until `shadow_n ≥ 5` and `dispatch_n ≥ 16`.

### 11.3 Metric pick list

| Goal | Metric |
|------|--------|
| Daily alert / sidebar | `dispatch_overhead_pct` |
| Sampling cost | `sampled_overhead_pct` |
| Legacy / soak | `hook_tax_pct` (blended, conservative) |

### 11.4 Tuning

```bash
PROBING_TORCH_PROFILING=on,shadow=8:2,rate=0.05   # more shadow points per window
```

### 11.5 Future work

Trimmed mean, per-cadence aggregation, EWMA sidebar, confidence bands, NCCL in-run shadow.

---

## 12. Operations

```bash
PROBING=1 PROBING_TORCH_PROFILING=on python train.py
PROBING=1 python examples/torch_probe_overhead_smoke.py
PROBING=1 make bench-quick
```

Reduce overhead: lower `rate` / `layer_rate`; disable `trace_spans`, `sync=on`, `backward=on`; use `shadow=off` only when in-run estimates are not needed.

---

## 13. Implementation index

| Component | Path |
|-----------|------|
| Shadow + timing | `python/probing/profiling/torch_probe.py` |
| Offline bench | `examples/bench_instrumentation.py` |
| Web SQL | `web/src/overhead/sql.rs` |
| Web formatting | `web/src/overhead/metrics.rs` |
| Skill SQL | `skills/health_overview/steps.yaml` |
| soak | `examples/soak_assert.py` |
| NCCL E2E | `examples/nccl_profiler_overhead.py` |

---

## Related docs

- [Profiling](profiling.md)
- [Tracing spans](tracing-spans.md)
- [NCCL Profiler](nccl-profiler.md)
- [Troubleshooting — High Overhead](../guide/troubleshooting.md)

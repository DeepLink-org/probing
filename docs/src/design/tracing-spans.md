# Span API

!!! note "Language"
    The full design is in **[中文 / Chinese](/zh/design/tracing-spans/)**.
    This page summarizes the public surface for English readers.

Training timelines use the **Span API**; module-level profiling uses **TorchProbe**
(`python.torch_trace`). See [Training Phases](/zh/design/training-phase/) for phase
invariants.

## Which API to use

| API | Stack | Persist | Use when |
|-----|-------|---------|----------|
| `attach_training_phases(model, optimizer)` | indirect | yes | **Default** — automatic forward/backward/optimizer + `train.step` |
| `with probing.span(...)` | yes | on exit (deferred) | Custom phases, nesting, `probing.event()` |
| `probing.record_span(name, duration_ns=...)` | no | immediately | Closed intervals with known duration |
| Raw `Span(...)` | yes | **no** | Internal only — use `probing.span` |

## Disabling persistence

- `PROBING_SPAN_BACKENDS=none` — stack only, no mmap writes
- `probing.tracing.configure_backends([])` — same until `reset_backends()`

Unset or empty env still defaults to `memtable`.

## Deferred close

`with probing.span` does not write rows on enter. Rows appear on exit, or when the
first `event()` forces a lazy `span_start`. In-flight spans are not visible in SQL
until then.

## Environment

| Variable | Default | Notes |
|----------|---------|-------|
| `PROBING_SPAN_BACKENDS` | `memtable` | `memtable`, `logger`, `otel`, `none` |
| `PROBING_SPAN_LOCATION` | off | `inspect.stack()` per span — expensive |

See [Environment Variables](../reference/env-vars.md#tracing--spans).

## Related

- [Training Phases](/zh/design/training-phase/) *(中文)*
- [Profiling](profiling.md)
- [Core Concepts](../guide/concepts.md)

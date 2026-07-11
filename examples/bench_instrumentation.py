#!/usr/bin/env python3
"""Benchmark probing instrumentation overhead — one-shot report.

Measures wall-clock cost of tracing spans, phase hooks, and TorchProbe at
representative settings. Run inside a probing-injected process::

    PROBING=1 python examples/bench_instrumentation.py

Quick smoke (fewer iterations)::

    PROBING=1 python examples/bench_instrumentation.py --quick

Optional JSON export::

    PROBING=1 python examples/bench_instrumentation.py --json-out /tmp/bench.json

NCCL profiler AllReduce overhead is **not** included here (needs multi-GPU +
``examples/run_nccl_profiler_bench.sh``). CPU pprof (SIGPROF) is server-side;
enable via config and compare with ``probing.pprof.sample_freq=0``.
"""

from __future__ import annotations

import argparse
import json
import platform
import statistics
import sys
import time
from dataclasses import asdict, dataclass, field
from typing import Callable, List, Optional

import probing
from probing.profiling.torch_probe import (
    DelayedRecord,
    TorchProbe,
    TorchProbeConfig,
    TorchStepTiming,
    shadow_step_in_cycle,
)
from probing.tracing import TraceEvent, bind_table, reset_backends
from probing.tracing.backends import configure as configure_backends
from probing.tracing.phases import BACKWARD, FORWARD, OPTIMIZER


@dataclass
class BenchRow:
    name: str
    group: str
    median_sec: float
    iterations: int
    vs_baseline_pct: Optional[float] = None
    per_iter_us: Optional[float] = None
    extra: dict = field(default_factory=dict)
    note: str = ""


@dataclass
class BenchReport:
    platform: str
    python: str
    probing_version: str
    torch: Optional[str]
    cuda: bool
    rows: List[BenchRow] = field(default_factory=list)

    def to_dict(self) -> dict:
        return {
            "platform": self.platform,
            "python": self.python,
            "probing_version": self.probing_version,
            "torch": self.torch,
            "cuda": self.cuda,
            "rows": [asdict(r) for r in self.rows],
        }


def _median_secs(fn: Callable[[], None], *, warmup: int, runs: int) -> float:
    for _ in range(warmup):
        fn()
    samples = []
    for _ in range(runs):
        t0 = time.perf_counter()
        fn()
        samples.append(time.perf_counter() - t0)
    return statistics.median(samples)


def _row(
    name: str,
    group: str,
    med: float,
    iterations: int,
    *,
    baseline: Optional[float] = None,
    extra: Optional[dict] = None,
    note: str = "",
) -> BenchRow:
    return BenchRow(
        name=name,
        group=group,
        median_sec=med,
        iterations=iterations,
        per_iter_us=(med / iterations) * 1e6 if iterations else None,
        vs_baseline_pct=_pct_vs(baseline, med) if baseline is not None else None,
        extra=extra or {},
        note=note,
    )


def _pct_vs(baseline: float, measured: float) -> Optional[float]:
    if baseline <= 0:
        return None
    return (measured / baseline - 1.0) * 100.0


def _init_tracing_tables() -> None:
    reset_backends(clear_registered=True)
    probing.step(0)
    probing.step(micro_batches=1)
    try:
        TraceEvent.drop()
    except Exception:
        pass
    TraceEvent.init_table()
    bind_table(TraceEvent)


class _FakeMod:
    pass


def _make_tracer(
    spec: str, *, live_sampling: bool = False
) -> tuple[TorchProbe, _FakeMod]:
    cfg = TorchProbeConfig.parse(spec)
    tracer = TorchProbe(config=cfg)
    tracer.has_backend = False
    root = _FakeMod()
    tracer.mod_names = {id(root): "model"}
    tracer._open_spans = {}
    tracer.pending = []
    tracer.events = {}
    tracer.cpu_start = {}
    tracer._step_cycle = 0
    if live_sampling:
        tracer.finalize_discovery()
    else:
        tracer.finalized = True
        tracer.sampled_step = True
    tracer._refresh_shadow_flag()
    return tracer, root


def _synthetic_torch_probe_steps(
    tracer: TorchProbe, root: _FakeMod, *, steps: int, live_sampling: bool = False
) -> list[float]:
    timings: list[float] = []
    orig_timing_save = TorchStepTiming.save
    orig_delayed_save = DelayedRecord.save

    def _capture_timing(self):
        timings.append(self.step_duration_sec)

    try:
        TorchStepTiming.save = _capture_timing  # type: ignore[method-assign]
        DelayedRecord.save = lambda self: None  # type: ignore[method-assign]
        probing.step(0)
        if not tracer.finalized:
            tracer.post_step_hook(None, (), {})
        for _ in range(steps):
            if tracer.finalized:
                tracer._mark_step_wall_start()
            if not tracer.shadow_step:
                if live_sampling:
                    tracer._ensure_step_plan()
                if (not live_sampling) or tracer._hooks_dispatch_active():
                    tracer.log_module_stage("pre forward", root)
                    tracer.log_module_stage("post forward", root)
            tracer.post_step_hook(None, (), {})
    finally:
        TorchStepTiming.save = orig_timing_save  # type: ignore[method-assign]
        DelayedRecord.save = orig_delayed_save  # type: ignore[method-assign]

    return timings


def bench_tracing_layer(*, span_iters: int, warmup: int, runs: int) -> list[BenchRow]:
    rows: list[BenchRow] = []

    def empty_loop():
        acc = 0
        for i in range(span_iters):
            acc += i & 1
        return acc

    def span_loop(backends: list[str]):
        configure_backends(backends)
        probing.step(0)

        def _run():
            for i in range(span_iters):
                with probing.span(f"iter-{i}", phase=FORWARD, source="bench"):
                    pass

        return _run

    def span_events_loop():
        configure_backends(["memtable"])
        probing.step(0)

        def _run():
            for i in range(span_iters):
                with probing.span(f"iter-{i}", phase=FORWARD, source="bench"):
                    probing.event("tick", attributes=[{"i": i}])

        return _run

    train_step_iters = max(1, span_iters // 10)

    def train_step_spans():
        configure_backends(["memtable"])
        probing.step(0)

        def _run():
            for _ in range(train_step_iters):
                with probing.span("forward", phase=FORWARD):
                    pass
                with probing.span("backward", phase=BACKWARD):
                    pass
                with probing.span("optimizer", phase=OPTIMIZER):
                    probing.step()

        return _run

    scenarios: list[tuple[str, Callable[[], None], int, dict]] = [
        ("empty loop", empty_loop, span_iters, {}),
        ("span (no backend)", span_loop([]), span_iters, {"backends": "none"}),
        (
            "span (memtable)",
            span_loop(["memtable"]),
            span_iters,
            {"backends": "memtable"},
        ),
        ("span + event (memtable)", span_events_loop(), span_iters, {}),
        ("train.step span triple", train_step_spans(), train_step_iters * 3, {}),
    ]

    measured: dict[str, float] = {}
    for name, fn, iters, extra in scenarios:
        measured[name] = _median_secs(fn, warmup=warmup, runs=runs)

    span_base = measured["span (no backend)"]
    for name, _, iters, extra in scenarios:
        rows.append(
            _row(
                name,
                "tracing",
                measured[name],
                iters,
                baseline=(
                    span_base
                    if name not in ("empty loop", "span (no backend)")
                    else None
                ),
                extra=extra,
            )
        )
    return rows


def bench_torch_probe_layer(*, steps: int) -> list[BenchRow]:
    rows: list[BenchRow] = []
    configure_backends(["memtable"])

    specs: list[tuple[str, str, bool]] = [
        (
            "hooks only (trace_spans=off, shadow=off)",
            "on,trace_spans=off,shadow=off",
            False,
        ),
        ("hooks + module spans", "on,trace_spans=on,shadow=off", False),
        ("default shadow 4:1", "on,trace_spans=off,shadow=4:1", False),
        ("sample rate 0.05", "on,trace_spans=off,shadow=off,rate=0.05", True),
        ("sample rate 1.0", "on,trace_spans=off,shadow=off,rate=1.0", True),
    ]

    medians: dict[str, float] = {}
    extras: dict[str, dict] = {}

    for label, spec, live_sampling in specs:
        tracer, root = _make_tracer(spec, live_sampling=live_sampling)
        timings = _synthetic_torch_probe_steps(
            tracer, root, steps=steps, live_sampling=live_sampling
        )
        med = statistics.median(timings) if timings else 0.0
        medians[label] = med
        extra: dict = {"steps": steps, "spec": spec}
        if "shadow=4:1" in spec and timings:
            probed = [t for i, t in enumerate(timings) if not shadow_step_in_cycle(i)]
            shadow = [t for i, t in enumerate(timings) if shadow_step_in_cycle(i)]
            if probed and shadow:
                tax = (statistics.median(probed) / statistics.median(shadow) - 1) * 100
                extra["hook_tax_pct"] = round(tax, 2)
        extras[label] = extra

    baseline = medians.get("hooks only (trace_spans=off, shadow=off)", 0.0)
    for label, med in medians.items():
        rows.append(
            _row(
                label,
                "torch_probe",
                med,
                steps,
                baseline=baseline if label != specs[0][0] else None,
                extra=extras.get(label, {}),
                note="per optimizer step (synthetic)",
            )
        )
    return rows


def _torch_info() -> tuple[Optional[str], bool]:
    try:
        import torch

        cuda = bool(torch.cuda.is_available())
        return torch.__version__, cuda
    except ImportError:
        return None, False


def _torch_warmup() -> None:
    import torch
    import torch.nn as nn

    m = nn.Linear(8, 4)
    x = torch.randn(16, 8)
    y = torch.randint(0, 4, (16,))
    opt = torch.optim.SGD(m.parameters(), lr=0.01)
    for _ in range(8):
        opt.zero_grad()
        loss = nn.functional.cross_entropy(m(x), y)
        loss.backward()
        opt.step()


def bench_torch_training_layer(
    *, batches: int, warmup: int, runs: int
) -> list[BenchRow]:
    import torch
    import torch.nn as nn
    import torch.nn.functional as F

    from probing.profiling.torch import install_hooks
    from probing.tracing.hooks import detach_training_phases

    device = torch.device("cuda" if torch.cuda.is_available() else "cpu")
    _torch_warmup()

    class TinyNet(nn.Module):
        def __init__(self) -> None:
            super().__init__()
            self.fc1 = nn.Linear(8, 16)
            self.fc2 = nn.Linear(16, 4)

        def forward(self, x: torch.Tensor) -> torch.Tensor:
            return self.fc2(F.relu(self.fc1(x)))

    def one_train_loop(*, phases: bool, torch_probe_spec: Optional[str]) -> float:
        """Return wall seconds for ``batches`` timed steps (after 2 warmup steps)."""
        model = TinyNet().to(device)
        opt = torch.optim.SGD(model.parameters(), lr=0.01)
        if phases:
            probing.attach_training_phases(model, opt)
        tracer = None
        if torch_probe_spec:
            cfg = TorchProbeConfig.parse(torch_probe_spec)
            tracer = TorchProbe(config=cfg)
            install_hooks(model, tracer=tracer)
            install_hooks(opt=opt, tracer=tracer)
        x = torch.randn(16, 8, device=device)
        y = torch.randint(0, 4, (16,), device=device)
        probing.step(0)
        for _ in range(2):
            logits = model(x)
            loss = F.cross_entropy(logits, y)
            opt.zero_grad()
            loss.backward()
            opt.step()
        t0 = time.perf_counter()
        for _ in range(batches):
            logits = model(x)
            loss = F.cross_entropy(logits, y)
            opt.zero_grad()
            loss.backward()
            opt.step()
        elapsed = time.perf_counter() - t0
        if phases:
            detach_training_phases(model, opt)
        if tracer is not None:
            from probing.profiling.torch import uninstall_hooks

            uninstall_hooks()
        return elapsed

    scenarios = [
        ("train baseline (no hooks)", None, False),
        ("train + phase hooks", None, True),
        (
            "train + TorchProbe (on, shadow=off)",
            "on,trace_spans=off,shadow=off,rate=1.0",
            False,
        ),
        (
            "train + phase hooks + TorchProbe",
            "on,trace_spans=on,shadow=4:1,rate=0.05",
            True,
        ),
    ]

    rows: list[BenchRow] = []
    configure_backends(["memtable"])
    baseline_name, _, _ = scenarios[0]
    base_samples: list[float] = []
    paired_delta: dict[str, list[float]] = {name: [] for name, _, _ in scenarios[1:]}
    rounds = warmup + runs

    for r in range(rounds):
        for name, probe_spec, phases in scenarios[1:]:
            base_t = one_train_loop(phases=False, torch_probe_spec=None)
            inst_t = one_train_loop(phases=phases, torch_probe_spec=probe_spec)
            if r >= warmup:
                base_samples.append(base_t)
                paired_delta[name].append(inst_t - base_t)

    base_med = statistics.median(base_samples)
    rows.append(
        _row(
            baseline_name,
            "torch_train",
            base_med,
            batches,
            extra={"device": str(device), "batches": batches},
            note="A/B paired baseline (back-to-back)",
        )
    )
    for name, _, _ in scenarios[1:]:
        delta_med = statistics.median(paired_delta[name])
        inst_med = base_med + delta_med
        rows.append(
            _row(
                name,
                "torch_train",
                inst_med,
                batches,
                baseline=base_med,
                extra={
                    "device": str(device),
                    "batches": batches,
                    "paired_delta_ms": round(delta_med * 1000, 3),
                },
                note=f"paired Δ {delta_med * 1000:+.2f}ms vs baseline",
            )
        )
    return rows


def _format_row_line(row: BenchRow) -> str:
    med_ms = row.median_sec * 1000.0
    per_us = f"{row.per_iter_us:.1f}µs" if row.per_iter_us is not None else "—"
    vs = (
        f"{row.vs_baseline_pct:+.1f}%"
        if row.vs_baseline_pct is not None and abs(row.vs_baseline_pct) < 10000
        else ("—" if row.vs_baseline_pct is None else "large")
    )
    note = row.note
    if row.extra.get("hook_tax_pct") is not None:
        note = f"hook_tax={row.extra['hook_tax_pct']}% {note}".strip()
    return f"{row.name:<42} {per_us:>10} {med_ms:>9.2f}ms {vs:>10}  {note}"


def _print_report_header(report: BenchReport) -> None:
    print("=" * 72)
    print("Probing instrumentation overhead report")
    print("=" * 72)
    print(f"Platform : {report.platform}")
    print(f"Python   : {report.python}")
    print(f"Probing  : {report.probing_version}")
    print(f"PyTorch  : {report.torch or '(not installed)'}")
    print(f"CUDA     : {report.cuda}")
    print()


def _print_group_table(rows: list[BenchRow], *, group: str) -> None:
    if not rows:
        return
    print(f"## {group}")
    print(f"{'scenario':<42} {'per-iter':>10} {'total':>10} {'vs base':>10}  notes")
    print("-" * 72)
    for row in rows:
        print(_format_row_line(row))
    print()


def _print_report_footer() -> None:
    print("Tips:")
    print("  • TorchProbe hook_tax compares probed vs shadow steps (shadow=4:1).")
    print(
        "  • NCCL profiler: make nccl-profiler-bench  or  examples/run_nccl_profiler_bench.sh"
    )
    print("  • SQL: SELECT * FROM python.torch_step_timing; python.trace_event")
    print("=" * 72)


def _print_report(report: BenchReport) -> None:
    _print_report_header(report)
    current_group = None
    group_rows: list[BenchRow] = []
    for row in report.rows:
        if row.group != current_group:
            if group_rows:
                _print_group_table(group_rows, group=current_group or "")
            current_group = row.group
            group_rows = []
        group_rows.append(row)
    if group_rows and current_group:
        _print_group_table(group_rows, group=current_group)
    _print_report_footer()


def _run_bench_group(
    report: BenchReport,
    *,
    index: int,
    total: int,
    group: str,
    label: str,
    fn: Callable[[], list[BenchRow]],
) -> list[BenchRow]:
    print(f">>> [{index}/{total}] {label} …", flush=True)
    rows = fn()
    report.rows.extend(rows)
    _print_group_table(rows, group=group)
    return rows


def main(argv: Optional[list[str]] = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--quick",
        action="store_true",
        help="Fewer iterations (faster, noisier)",
    )
    parser.add_argument(
        "--json-out",
        type=str,
        default="",
        help="Write machine-readable report to this path",
    )
    parser.add_argument(
        "--skip-torch",
        action="store_true",
        help="Skip PyTorch training micro-benchmarks",
    )
    parser.add_argument(
        "--batch",
        action="store_true",
        help="Print the full report once at the end (default: emit each group as it finishes)",
    )
    args = parser.parse_args(argv)

    if args.quick:
        span_iters, probe_steps, batches, warmup, runs = 80, 12, 8, 1, 3
    else:
        span_iters, probe_steps, batches, warmup, runs = 300, 40, 30, 2, 5

    torch_ver, cuda = _torch_info()
    report = BenchReport(
        platform=platform.platform(),
        python=sys.version.split()[0],
        probing_version=getattr(probing, "VERSION", "unknown"),
        torch=torch_ver,
        cuda=cuda,
    )

    _init_tracing_tables()

    def torch_train_rows() -> list[BenchRow]:
        if not args.skip_torch and torch_ver:
            return bench_torch_training_layer(batches=batches, warmup=warmup, runs=runs)
        if not torch_ver:
            return [
                BenchRow(
                    name="torch training (skipped)",
                    group="torch_train",
                    median_sec=0.0,
                    iterations=0,
                    note="install torch to enable",
                )
            ]
        return []

    bench_plan: list[tuple[str, str, Callable[[], list[BenchRow]]]] = [
        (
            "tracing",
            "Tracing spans (memtable / events)",
            lambda: bench_tracing_layer(
                span_iters=span_iters, warmup=warmup, runs=runs
            ),
        ),
        (
            "torch_probe",
            "TorchProbe synthetic hooks",
            lambda: bench_torch_probe_layer(steps=probe_steps),
        ),
        (
            "torch_train",
            "PyTorch TinyNet training (A/B paired)",
            torch_train_rows,
        ),
    ]
    if args.skip_torch or not torch_ver:
        bench_plan = [p for p in bench_plan if p[0] != "torch_train"]
        if not args.skip_torch and not torch_ver:
            bench_plan.append(
                (
                    "torch_train",
                    "PyTorch training (skipped — no torch)",
                    torch_train_rows,
                )
            )

    if args.batch:
        for _, _, fn in bench_plan:
            report.rows.extend(fn())
        _print_report(report)
    else:
        _print_report_header(report)
        total = len(bench_plan)
        for i, (group, label, fn) in enumerate(bench_plan, start=1):
            _run_bench_group(
                report,
                index=i,
                total=total,
                group=group,
                label=label,
                fn=fn,
            )
        _print_report_footer()

    if args.json_out:
        with open(args.json_out, "w", encoding="utf-8") as fh:
            json.dump(report.to_dict(), fh, indent=2)
        print(f"Wrote {args.json_out}")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
"""Post-soak assertions for long-running training + probing CI jobs.

Run inside a probing-injected process after a soak workload, or standalone
against persisted memtable data (``PROBING_DATA_DIR``)::

    PROBING=1 python examples/soak_assert.py --require-torch-profiling

Exit code 0 on success, 1 on assertion failure.
"""

from __future__ import annotations

import argparse
import os
import sys
from dataclasses import dataclass, field
from typing import Any, Optional


@dataclass
class SoakAssertConfig:
    min_trace_event: int = 1
    min_torch_trace: int = 1
    min_torch_step_timing: int = 1
    min_steps: int = 1
    max_hook_tax_pct: float = 75.0
    require_torch_profiling: bool = False


@dataclass
class SoakAssertResult:
    ok: bool = True
    failures: list[str] = field(default_factory=list)
    notes: list[str] = field(default_factory=list)

    def fail(self, msg: str) -> None:
        self.ok = False
        self.failures.append(msg)

    def note(self, msg: str) -> None:
        self.notes.append(msg)


def _scalar(df: Any, default: Optional[float] = None) -> Optional[float]:
    if df is None or getattr(df, "empty", True):
        return default
    try:
        val = df.iloc[0, 0]
    except Exception:
        return default
    if val is None:
        return default
    try:
        return float(val)
    except (TypeError, ValueError):
        return default


def _count_rows(sql: str) -> int:
    import probing

    df = probing.query(sql)
    if df is None or getattr(df, "empty", True):
        return 0
    val = _scalar(df, default=0.0)
    return int(val) if val is not None else 0


def _table_exists(schema_table: str) -> bool:
    import probing

    try:
        df = probing.query(f"SELECT 1 FROM {schema_table} LIMIT 1")
    except Exception:
        return False
    return df is not None


def run_assertions(cfg: SoakAssertConfig) -> SoakAssertResult:
    result = SoakAssertResult()

    try:
        import probing  # noqa: F401
    except ImportError:
        result.fail("probing is not installed")
        return result

    trace_events = _count_rows("SELECT count(*) FROM python.trace_event")
    result.note(f"python.trace_event rows={trace_events}")
    if trace_events < cfg.min_trace_event:
        result.fail(
            f"expected >= {cfg.min_trace_event} trace_event rows, got {trace_events}"
        )

    timing_rows = _count_rows("SELECT count(*) FROM python.torch_step_timing")
    torch_rows = _count_rows("SELECT count(*) FROM python.torch_trace")
    result.note(f"python.torch_step_timing rows={timing_rows}")
    result.note(f"python.torch_trace rows={torch_rows}")

    profiling_active = cfg.require_torch_profiling or timing_rows > 0 or torch_rows > 0
    if cfg.require_torch_profiling and not profiling_active:
        result.fail(
            "PROBING_TORCH_PROFILING was required but torch_trace/torch_step_timing are empty"
        )

    if profiling_active:
        if torch_rows < cfg.min_torch_trace:
            result.fail(
                f"expected >= {cfg.min_torch_trace} torch_trace rows, got {torch_rows}"
            )
        if timing_rows < cfg.min_torch_step_timing:
            result.fail(
                f"expected >= {cfg.min_torch_step_timing} torch_step_timing rows, "
                f"got {timing_rows}"
            )

        step_count = _count_rows(
            "SELECT count(*) FROM python.torch_step_timing WHERE local_step > 0"
        )
        if step_count < cfg.min_steps:
            result.fail(
                f"expected >= {cfg.min_steps} optimizer steps in torch_step_timing, "
                f"got {step_count}"
            )

        hook_tax = _scalar(
            probing_query(
                """
                SELECT round(
                  (median(CASE WHEN is_shadow = 0 THEN step_duration_sec END)
                   / nullif(median(CASE WHEN is_shadow = 1 THEN step_duration_sec END), 0)
                   - 1) * 100, 2
                ) AS hook_tax_pct
                FROM python.torch_step_timing
                WHERE local_step > 0
                """
            )
        )
        shadow_n = _count_rows(
            "SELECT count(*) FROM python.torch_step_timing "
            "WHERE local_step > 0 AND is_shadow = 1"
        )
        probed_n = _count_rows(
            "SELECT count(*) FROM python.torch_step_timing "
            "WHERE local_step > 0 AND is_shadow = 0"
        )
        result.note(
            f"overhead: hook_tax_pct={hook_tax} probed_n={probed_n} shadow_n={shadow_n}"
        )
        if shadow_n > 0 and probed_n > 0 and hook_tax is not None:
            if hook_tax > cfg.max_hook_tax_pct:
                result.fail(
                    f"hook tax {hook_tax:.1f}% exceeds max {cfg.max_hook_tax_pct}%"
                )
        elif shadow_n == 0 or probed_n == 0:
            result.note("overhead ratio skipped (need both probed and shadow steps)")

    # Optional cluster tables when torchrun cluster is active.
    if os.environ.get("SOAK_EXPECT_CLUSTER"):
        if _table_exists("cluster.nodes"):
            nodes = _count_rows("SELECT count(*) FROM cluster.nodes")
            result.note(f"cluster.nodes rows={nodes}")
            if nodes < 1:
                result.fail("cluster.nodes is empty after distributed soak")
        else:
            result.fail("cluster.nodes table missing in distributed soak")

    return result


def probing_query(sql: str) -> Any:
    import probing

    return probing.query(sql)


def _build_config(args: argparse.Namespace) -> SoakAssertConfig:
    return SoakAssertConfig(
        min_trace_event=args.min_trace_event,
        min_torch_trace=args.min_torch_trace,
        min_torch_step_timing=args.min_torch_step_timing,
        min_steps=args.min_steps,
        max_hook_tax_pct=args.max_hook_tax_pct,
        require_torch_profiling=args.require_torch_profiling,
    )


def main(argv: Optional[list[str]] = None) -> int:
    parser = argparse.ArgumentParser(description="Assert probing soak run health")
    parser.add_argument(
        "--min-trace-event",
        type=int,
        default=1,
        help="minimum rows in python.trace_event",
    )
    parser.add_argument(
        "--min-torch-trace",
        type=int,
        default=1,
        help="minimum rows in python.torch_trace when profiling is active",
    )
    parser.add_argument(
        "--min-torch-step-timing",
        type=int,
        default=1,
        help="minimum rows in python.torch_step_timing when profiling is active",
    )
    parser.add_argument(
        "--min-steps",
        type=int,
        default=1,
        help="minimum optimizer steps recorded in torch_step_timing",
    )
    parser.add_argument(
        "--max-hook-tax-pct",
        type=float,
        default=75.0,
        help="fail when median probed/shadow step overhead exceeds this percent",
    )
    parser.add_argument(
        "--require-torch-profiling",
        action="store_true",
        help="fail when torch_trace / torch_step_timing tables are empty",
    )
    args = parser.parse_args(argv)
    cfg = _build_config(args)
    result = run_assertions(cfg)

    for line in result.notes:
        print(f"soak_assert: {line}")
    if result.ok:
        print("soak_assert: OK")
        return 0
    for line in result.failures:
        print(f"soak_assert: FAIL: {line}", file=sys.stderr)
    return 1


if __name__ == "__main__":
    raise SystemExit(main())

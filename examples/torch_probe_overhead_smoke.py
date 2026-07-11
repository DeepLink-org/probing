#!/usr/bin/env python3
"""Smoke-check TorchProbe in-run overhead (shadow step + torch_step_timing).

No GPU or torch.distributed required. Run inside a probing-injected process::

    PROBING=1 python examples/torch_probe_overhead_smoke.py

Or from the repo with the dev venv::

    .venv/bin/python examples/torch_probe_overhead_smoke.py
"""

from __future__ import annotations

import sys
import time

from probing.profiling.torch_probe import TorchProbe, TorchProbeConfig


class _FakeMod:
    pass


def main() -> int:
    tracer = TorchProbe(TorchProbeConfig.parse("on,shadow=4:1"))
    tracer.finalized = True
    tracer.sampled_step = True
    tracer.has_backend = False
    root = _FakeMod()
    tracer.mod_names = {id(root): "model"}
    tracer._step_cycle = 0
    tracer._refresh_shadow_flag()
    tracer._mark_step_wall_start()

    print("Running 10 synthetic optimizer steps (shadow cadence 4:1)…")
    for step in range(10):
        if not tracer.shadow_step:
            tracer.log_module_stage("pre forward", root)
            time.sleep(0.001)
            tracer.log_module_stage("post forward", root)
        tracer.post_step_hook(None, (), {})
        print(f"  step {step + 1}: shadow={tracer.shadow_step}")

    print()
    print("Query overhead (requires probing SQL engine in-process):")
    print(
        """
SELECT
  round((median(CASE WHEN is_shadow = 0 THEN step_duration_sec END)
        / nullif(median(CASE WHEN is_shadow = 1 THEN step_duration_sec END), 0) - 1) * 100, 2)
    AS hook_tax_pct,
  sum(CASE WHEN is_shadow = 1 THEN 1 ELSE 0 END) AS shadow_n
FROM python.torch_step_timing
WHERE local_step > 0;
""".strip()
    )

    try:
        import probing

        df = probing.query(
            "SELECT is_shadow, sampled, step_duration_sec, sample_rate, sample_mode "
            "FROM python.torch_step_timing ORDER BY local_step"
        )
        print(df)
    except Exception as exc:
        print(f"(SQL query skipped: {exc})", file=sys.stderr)
        print(
            "Rows were still written if TorchStepTiming.save succeeded.",
            file=sys.stderr,
        )

    return 0


if __name__ == "__main__":
    raise SystemExit(main())

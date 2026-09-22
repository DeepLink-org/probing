"""Install the Probing XTuner adapter, then hand over to XTuner's RL CLI.

Installing here covers the driver process, which is where the trainer and its
hook points live. Rollout workers are separate Ray actors and are not touched.
"""

from __future__ import annotations


def _install_adapter() -> None:
    try:
        from probing.ext import xtuner as adapter

        adapter.install()
        print("[probing] xtuner adapter installed", flush=True)
    except Exception as exc:
        print(f"[probing] xtuner adapter install skipped: {exc}", flush=True)


def main() -> None:
    _install_adapter()
    from xtuner.v1.train.cli.rl import app

    app(exit_on_error=False)


if __name__ == "__main__":
    main()

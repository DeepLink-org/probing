"""CLI: ``python -m probing.fakes [install|uninstall|status|loop|pretrain_gpt]``."""

from __future__ import annotations

import argparse
import json
import sys


def main(argv: list[str] | None = None) -> int:
    argv = list(sys.argv[1:] if argv is None else argv)
    # Allow ``python -m probing.fakes pretrain_gpt --train-iters 4``
    if argv and argv[0] == "pretrain_gpt":
        # Prefer the examples script so CLI flags stay in one place.
        from probing.fakes.megatron_lm import probing_repo_root

        root = probing_repo_root()
        script = root / "examples" / "pretrain_gpt.py"
        if script.is_file():
            import runpy

            sys.argv = [str(script), *argv[1:]]
            runpy.run_path(str(script), run_name="__main__")
            return 0
        # Fallback: in-package minimal runner
        from probing.fakes.specs import megatron as megatron_spec
        from probing.fakes import install, run_pretrain

        install(force=True)
        megatron_spec.set_args(megatron_spec.default_args(train_iters=4))
        megatron_spec.ensure_sys_modules()
        result = run_pretrain()
        print(result)
        return 0

    parser = argparse.ArgumentParser(
        prog="python -m probing.fakes",
        description="Fake CUDA/Megatron packages for macOS meta-device debugging",
    )
    parser.add_argument(
        "command",
        nargs="?",
        default="status",
        choices=("install", "uninstall", "status", "loop", "pretrain_gpt"),
    )
    parser.add_argument("--steps", type=int, default=4, help="scripted loop steps")
    parser.add_argument("--tp", type=int, default=0)
    parser.add_argument("--pp", type=int, default=0)
    parser.add_argument("--dp", type=int, default=0)
    parser.add_argument("--device", default=None, help="meta|cpu|mps")
    parser.add_argument(
        "--force",
        action="store_true",
        help="shadow real packages (or set PROBING_FAKES_FORCE=1)",
    )
    args, _unknown = parser.parse_known_args(argv)

    from probing import fakes

    if args.command == "install":
        if args.force:
            import os

            os.environ["PROBING_FAKES_FORCE"] = "1"
        fakes.maybe_install_from_env() or fakes.install(
            device=args.device, force=args.force or None
        )
        print("installed", sorted(fakes.registered_specs()))
        print("force", args.force)
        return 0

    if args.command == "uninstall":
        fakes.uninstall()
        print("uninstalled")
        return 0

    if args.command == "status":
        print(
            json.dumps(
                {
                    "installed": fakes.is_installed(),
                    "device": fakes.target_device() if fakes.is_installed() else None,
                    "specs": sorted(fakes.registered_specs()),
                },
                indent=2,
            )
        )
        return 0

    if args.command == "loop":
        fakes.maybe_install_from_env() or fakes.install(device=args.device, force=True)
        result = fakes.run_scripted_loop(
            steps=args.steps,
            tp=args.tp,
            pp=args.pp,
            dp=args.dp,
            device=args.device,
        )
        print(
            json.dumps(
                {
                    "steps": result.steps,
                    "role": result.role,
                    "device": result.device,
                    "last_iteration": result.last_iteration,
                },
                indent=2,
            )
        )
        return 0

    parser.error(f"unknown command {args.command}")
    return 2


if __name__ == "__main__":
    sys.exit(main())

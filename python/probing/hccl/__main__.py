"""CLI: ``python -m probing.hccl --shim-dir``"""

from __future__ import annotations

import argparse
import sys

from probing.hccl import (
    ENV_REAL,
    ENV_SHIM_LOG,
    install_real_copy,
    ld_library_path_prefix,
    shim_dir,
    shim_path,
)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        prog="python -m probing.hccl",
        description="HCCL libprofapi.so shim utilities",
    )
    parser.add_argument(
        "--shim-path",
        action="store_true",
        help="print absolute path to libprofapi.so (probing shim)",
    )
    parser.add_argument(
        "--shim-dir",
        action="store_true",
        help="print directory to prepend to LD_LIBRARY_PATH",
    )
    parser.add_argument(
        "--install-real",
        metavar="PATH",
        help="copy CANN libprofapi.so to libprofapi.so.real next to the shim",
    )
    args = parser.parse_args(argv)

    if args.shim_path:
        try:
            print(shim_path())
        except (OSError, FileNotFoundError) as e:
            print(e, file=sys.stderr)
            return 1
        return 0

    if args.shim_dir:
        try:
            print(shim_dir())
        except (OSError, FileNotFoundError) as e:
            print(e, file=sys.stderr)
            return 1
        return 0

    if args.install_real:
        try:
            dest = install_real_copy(args.install_real)
        except (OSError, FileNotFoundError) as e:
            print(e, file=sys.stderr)
            return 1
        print(dest, file=sys.stderr)
        return 0

    parser.print_help(sys.stderr)
    print("\nExample:", file=sys.stderr)
    print(
        "  export LD_LIBRARY_PATH=$(python -m probing.hccl --shim-dir):$LD_LIBRARY_PATH",
        file=sys.stderr,
    )
    print(
        f"  export {ENV_REAL}=/path/to/cann/lib64/libprofapi.so  # optional if libprofapi.so.real present",
        file=sys.stderr,
    )
    print("  export PROBING=2", file=sys.stderr)
    print(f"  export {ENV_SHIM_LOG}=1  # optional shim debug log", file=sys.stderr)
    return 2


if __name__ == "__main__":
    raise SystemExit(main())

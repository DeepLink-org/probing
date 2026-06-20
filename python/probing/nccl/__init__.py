"""NCCL profiler plugin helpers (Linux, NCCL ≥ 2.26)."""

from __future__ import annotations

import os
import sys
from pathlib import Path

__all__ = ["plugin_path", "DEFAULT_EVENT_MASK", "seed_mock", "maybe_auto_seed"]

# Coll | ProxyOp | ProxyStep
DEFAULT_EVENT_MASK = 26

_LIB_BASENAME = "libprobing_nccl_profiler"
_ENV_OVERRIDE = "PROBING_NCCL_PROFILER"


def _lib_filename() -> str:
    if sys.platform == "linux":
        return f"{_LIB_BASENAME}.so"
    if sys.platform == "darwin":
        return f"{_LIB_BASENAME}.dylib"
    raise OSError(f"NCCL profiler plugin is not supported on {sys.platform}")


def _candidate_paths() -> list[Path]:
    pkg_root = Path(__file__).resolve().parent.parent
    name = _lib_filename()
    out = [
        pkg_root / "libs" / name,
    ]
    # Editable / source tree: repo target/ after `make nccl-profiler`
    repo_root = Path(__file__).resolve().parents[3]
    for profile in ("release", "debug"):
        out.append(repo_root / "target" / profile / name)
    return out


def plugin_path() -> str:
    """Absolute path to the NCCL profiler plugin shared library.

    Resolution order:
    1. ``PROBING_NCCL_PROFILER`` environment variable
    2. ``probing/libs/libprobing_nccl_profiler.so`` (wheel)
    3. ``target/release|debug/`` (local ``cargo build -p probing-nccl-profiler``)
    """
    override = os.environ.get(_ENV_OVERRIDE)
    if override:
        path = Path(override).expanduser().resolve()
        if not path.is_file():
            raise FileNotFoundError(
                f"{_ENV_OVERRIDE}={override!r} does not point to an existing file"
            )
        return str(path)

    if sys.platform != "linux":
        raise OSError(
            "NCCL profiler plugin is only available on Linux; "
            f"set {_ENV_OVERRIDE} if you have a custom build"
        )

    for candidate in _candidate_paths():
        if candidate.is_file():
            return str(candidate.resolve())

    searched = ", ".join(str(p) for p in _candidate_paths())
    raise FileNotFoundError(
        "NCCL profiler plugin not found. Build with "
        "`cargo build -p probing-nccl-profiler --release` or "
        f"set {_ENV_OVERRIDE}. Searched: {searched}"
    )


def __getattr__(name: str):
    if name in ("seed_mock", "maybe_auto_seed"):
        from probing.nccl.mock import maybe_auto_seed, seed_mock

        return seed_mock if name == "seed_mock" else maybe_auto_seed
    raise AttributeError(f"module {__name__!r} has no attribute {name!r}")

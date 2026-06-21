"""HCCL MSProf shim helpers (Linux, Ascend / CANN)."""

from __future__ import annotations

import os
import shutil
import sys
from pathlib import Path

__all__ = [
    "shim_path",
    "shim_dir",
    "ld_library_path_prefix",
    "install_real_copy",
    "ENV_REAL",
    "ENV_SHIM_LOG",
]

_LIB_BASENAME = "libprofapi.so"
_REAL_BASENAME = "libprofapi.so.real"
_ENV_OVERRIDE = "PROBING_HCCL_SHIM"
ENV_REAL = "PROBING_HCCL_PROFAPI_REAL"
ENV_SHIM_LOG = "PROBING_HCCL_SHIM_LOG"


def _candidate_paths() -> list[Path]:
    pkg_root = Path(__file__).resolve().parent.parent
    name = _LIB_BASENAME
    out = [
        pkg_root / "shim" / "hccl" / name,
    ]
    repo_root = Path(__file__).resolve().parents[3]
    for profile in ("release", "debug"):
        out.append(repo_root / "target" / profile / name)
    return out


def shim_path() -> str:
    """Absolute path to the probing ``libprofapi.so`` shim."""
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
            "HCCL MSProf shim is only available on Linux; "
            f"set {_ENV_OVERRIDE} if you have a custom build"
        )

    for candidate in _candidate_paths():
        if candidate.is_file():
            return str(candidate.resolve())

    searched = ", ".join(str(p) for p in _candidate_paths())
    raise FileNotFoundError(
        "HCCL shim not found. Build with "
        "`make hccl-shim-lib` or `cargo build -p probing-hccl-shim --release`, "
        f"or set {_ENV_OVERRIDE}. Searched: {searched}"
    )


def shim_dir() -> str:
    """Directory containing ``libprofapi.so`` (for ``LD_LIBRARY_PATH``)."""
    return str(Path(shim_path()).parent)


def ld_library_path_prefix() -> str:
    """Value to prepend to ``LD_LIBRARY_PATH`` (shim dir only)."""
    return shim_dir()


def install_real_copy(cann_libprofapi: str | Path) -> Path:
    """Copy CANN's real ``libprofapi.so`` to ``libprofapi.so.real`` beside the shim."""
    src = Path(cann_libprofapi).expanduser().resolve()
    if not src.is_file():
        raise FileNotFoundError(f"CANN libprofapi not found: {src}")
    dest = Path(shim_dir()) / _REAL_BASENAME
    dest.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(src, dest)
    return dest

"""Install the develop-mode ``probing_hook.pth`` (pairs with maturin ``probing.pth``).

``maturin develop`` writes ``probing.pth`` with only the repo ``python/`` path.
Wheel installs ``probing.pth`` with ``import probing_hook`` because the package
lives directly in site-packages. This module restores the same auto-hook behavior
for editable installs without fighting maturin's generated path file.
"""

from __future__ import annotations

import site
import sys
from pathlib import Path

HOOK_PTH_NAME = "probing_hook.pth"
HOOK_PTH_LINE = "import probing_hook\n"


def repo_python_dir() -> Path:
    return Path(__file__).resolve().parents[1]


def hook_pth_path() -> Path:
    return Path(site.getsitepackages()[0]) / HOOK_PTH_NAME


def install_dev_hook() -> Path:
    target = hook_pth_path()
    target.write_text(HOOK_PTH_LINE, encoding="utf-8")
    return target


def remove_dev_hook() -> bool:
    target = hook_pth_path()
    if target.is_file():
        target.unlink()
        return True
    return False


def is_dev_hook_installed() -> bool:
    target = hook_pth_path()
    return target.is_file() and target.read_text(encoding="utf-8") == HOOK_PTH_LINE


def main(argv: list[str] | None = None) -> int:
    argv = argv if argv is not None else sys.argv[1:]
    cmd = argv[0] if argv else "install"

    if cmd == "install":
        path = install_dev_hook()
        print(f"installed {path}")
        print(f"  python path via maturin probing.pth → {repo_python_dir()}")
        return 0

    if cmd == "remove":
        print("removed" if remove_dev_hook() else "not installed")
        return 0

    if cmd == "status":
        path = hook_pth_path()
        print(f"hook: {'ok' if is_dev_hook_installed() else 'missing'} ({path})")
        print(f"python: {repo_python_dir()}")
        return 0 if is_dev_hook_installed() else 1

    print("usage: python probing/dev_pth.py [install|remove|status]", file=sys.stderr)
    return 1


if __name__ == "__main__":
    raise SystemExit(main())

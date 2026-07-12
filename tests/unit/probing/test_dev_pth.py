"""Tests for develop-mode site hook installation."""

from __future__ import annotations

from probing.dev_pth import (
    HOOK_PTH_NAME,
    HOOK_PTH_LINE_PREFIX,
    hook_pth_line,
    hook_pth_path,
    install_dev_hook,
    is_dev_hook_installed,
    remove_dev_hook,
    repo_python_dir,
)


def test_repo_python_dir():
    root = repo_python_dir()
    assert root.is_dir()
    assert (root / "probing").is_dir()
    assert (root / "probing_hook.py").is_file()


def test_install_and_remove_dev_hook():
    remove_dev_hook()
    assert not is_dev_hook_installed()

    path = install_dev_hook()
    assert path == hook_pth_path()
    assert path.name == HOOK_PTH_NAME
    line = path.read_text(encoding="utf-8")
    assert line == hook_pth_line()
    assert line.startswith(HOOK_PTH_LINE_PREFIX)
    assert line.endswith("import probing_hook\n")
    assert is_dev_hook_installed()

    assert remove_dev_hook()
    assert not is_dev_hook_installed()

    # Restore for other tests / local dev when run inside activated venv.
    install_dev_hook()

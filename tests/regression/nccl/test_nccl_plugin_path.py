"""Tests for probing.nccl plugin path helpers."""

from __future__ import annotations

import os
import sys
from pathlib import Path
from unittest import mock

import pytest

from probing.nccl import DEFAULT_EVENT_MASK, plugin_path
from probing.nccl.__main__ import main


def test_default_event_mask():
    # Coll | P2P | ProxyOp | ProxyStep | KernelCh
    assert DEFAULT_EVENT_MASK == 94


def test_plugin_path_env_override(tmp_path):
    lib = tmp_path / "custom.so"
    lib.write_bytes(b"\x00")
    with mock.patch.dict(os.environ, {"PROBING_NCCL_PROFILER": str(lib)}):
        assert plugin_path() == str(lib.resolve())


def test_plugin_path_env_override_missing(tmp_path):
    missing = tmp_path / "nope.so"
    with mock.patch.dict(os.environ, {"PROBING_NCCL_PROFILER": str(missing)}):
        with pytest.raises(FileNotFoundError):
            plugin_path()


@pytest.mark.skipif(sys.platform != "linux", reason="bundled .so is Linux-only")
def test_plugin_path_finds_bundled_or_build():
    # In CI/dev after `make nccl-profiler-lib`, one of the candidates should exist.
    try:
        path = plugin_path()
    except FileNotFoundError:
        pytest.skip("nccl profiler .so not built")
    assert path.endswith("libprobing_nccl_profiler.so")
    assert Path(path).is_file()


def test_cli_plugin_path_env_override(tmp_path, capsys):
    lib = tmp_path / "custom.so"
    lib.write_bytes(b"\x00")
    with mock.patch.dict(os.environ, {"PROBING_NCCL_PROFILER": str(lib)}):
        assert main(["--plugin-path"]) == 0
    assert capsys.readouterr().out.strip() == str(lib.resolve())


def test_cli_help_exits_2(capsys):
    assert main([]) == 2
    assert "plugin-path" in capsys.readouterr().err


def test_cli_event_mask(capsys):
    assert main(["--event-mask"]) == 0
    assert capsys.readouterr().out.strip() == "94"


def test_cli_help_lists_seed_mock(capsys):
    assert main([]) == 2
    assert "--seed-mock" in capsys.readouterr().err

"""Unit tests for torchrun cluster helpers (Rust-backed; no torchrun required)."""

from __future__ import annotations

import os

import pytest


class TestPythonFacade:
    def test_setup_noop_when_disabled(self, monkeypatch):
        import probing.torchrun_cluster as tc

        tc._SETUP_DONE = False
        monkeypatch.setenv("PROBING_TORCHRUN_CLUSTER", "0")
        monkeypatch.setenv("WORLD_SIZE", "8")
        assert tc.setup_torchrun_cluster() is None

    def test_setup_noop_for_single_process(self, monkeypatch):
        import probing.torchrun_cluster as tc

        tc._SETUP_DONE = False
        monkeypatch.delenv("PROBING_TORCHRUN_CLUSTER", raising=False)
        monkeypatch.setenv("WORLD_SIZE", "1")
        assert tc.setup_torchrun_cluster() is None

    def test_setup_calls_rust_once(self, monkeypatch):
        import probing.torchrun_cluster as tc
        import probing._core as core

        tc._SETUP_DONE = False
        monkeypatch.setenv("WORLD_SIZE", "8")
        monkeypatch.delenv("PROBING_TORCHRUN_CLUSTER", raising=False)
        calls = {"n": 0}

        def fake_start():
            calls["n"] += 1
            return "http://127.0.0.1:18080"

        monkeypatch.setattr(core, "start_torchrun_cluster", fake_start)
        info = tc.setup_torchrun_cluster()
        assert info == {
            "http_base": "http://127.0.0.1:18080",
            "addr": "127.0.0.1:18080",
        }
        assert calls["n"] == 1
        assert tc.setup_torchrun_cluster() is None

    def test_refresh_node_role_syncs_env(self, monkeypatch):
        import probing.torchrun_cluster as tc
        import probing._core as core
        import probing.parallel as parallel

        parallel.clear_role()
        parallel.set_role(dp=2, pp=1)
        monkeypatch.setattr(core, "refresh_torchrun_cluster_role", lambda: True)
        assert tc.refresh_node_role() is True
        assert os.environ.get("PROBING_NODE_ROLE") == "dp=2,pp=1"
        parallel.clear_role()

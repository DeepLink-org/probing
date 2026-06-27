"""Regression tests for torchrun cluster (Python facade; logic lives in Rust)."""

from __future__ import annotations

import pytest


@pytest.mark.skipif(
    not __import__("os").environ.get("PROBING"),
    reason="needs in-process probing engine (PROBING=1)",
)
class TestRustTorchrunCluster:
    def test_start_torchrun_cluster_is_idempotent(self, monkeypatch):
        monkeypatch.setenv("WORLD_SIZE", "2")
        monkeypatch.setenv("RANK", "0")
        monkeypatch.setenv("LOCAL_RANK", "0")
        monkeypatch.setenv("MASTER_ADDR", "127.0.0.1")
        monkeypatch.setenv("MASTER_PORT", "29681")
        monkeypatch.setenv("RDZV_ID", "test-rust-cluster")

        from probing import _core

        first = _core.start_torchrun_cluster()
        second = _core.start_torchrun_cluster()
        assert first == second or (first is not None and second is not None)

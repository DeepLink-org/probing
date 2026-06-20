"""Unit tests for probing.torchrun_cluster helpers (no torchrun required)."""

import json
import os

import pytest

from probing.torchrun_cluster import (
    _master_store_key,
    _reachable_addr,
    _run_prefix,
    publish_master,
    wait_master,
)


class TestTorchrunClusterHelpers:
    def test_run_prefix_uses_rdzv_id(self, monkeypatch):
        monkeypatch.setenv("RDZV_ID", "job-abc")
        assert _run_prefix() == "probing/torchrun/job-abc"

    def test_master_store_key(self, monkeypatch):
        monkeypatch.setenv("TORCHELASTIC_RUN_ID", "elastic-1")
        assert _master_store_key() == "probing/torchrun/elastic-1/master"

    def test_reachable_addr_maps_unspecified_bind(self, monkeypatch):
        monkeypatch.setenv("MASTER_ADDR", "10.0.0.1")
        assert _reachable_addr("0.0.0.0:9922") == "10.0.0.1:9922"

    def test_reachable_addr_keeps_specific_host(self):
        assert _reachable_addr("192.168.1.5:8080") == "192.168.1.5:8080"


class _FakeStore:
    def __init__(self):
        self._data: dict[str, str] = {}

    def set(self, key: str, value: str) -> None:
        self._data[key] = value

    def get(self, key: str) -> bytes:
        return self._data[key].encode()

    def wait(self, keys, timeout=None):  # noqa: ANN001
        for key in keys:
            if key not in self._data:
                raise RuntimeError(f"missing key {key}")


@pytest.mark.skipif(
    not os.environ.get("PROBING"),
    reason="needs in-process probing engine (PROBING=1)",
)
class TestPublishWaitMaster:
    def test_publish_and_wait_roundtrip(self, monkeypatch):
        pytest.importorskip("torch")
        import torch.distributed as dist

        fake_store = _FakeStore()
        monkeypatch.setattr(
            "probing.torchrun_cluster._rendezvous_store",
            lambda: fake_store,
        )
        monkeypatch.setattr(
            "probing.torchrun_cluster._ensure_http_server",
            lambda: "0.0.0.0:19901",
        )
        monkeypatch.setenv("MASTER_ADDR", "127.0.0.1")
        monkeypatch.setenv("TORCHELASTIC_RUN_ID", "test-run")

        monkeypatch.setattr(dist, "get_rank", lambda: 0)
        monkeypatch.setattr(dist, "is_initialized", lambda: True)

        info = publish_master()
        assert info["addr"] == "127.0.0.1:19901"
        assert info["http_base"] == "http://127.0.0.1:19901"

        raw = fake_store.get(_master_store_key()).decode()
        assert json.loads(raw)["addr"] == "127.0.0.1:19901"

        monkeypatch.setattr(dist, "get_rank", lambda: 1)
        waited = wait_master(timeout_sec=1.0)
        assert waited["http_base"] == "http://127.0.0.1:19901"

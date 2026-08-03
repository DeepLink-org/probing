from __future__ import annotations

from probing.profiling import runtime_debug


class FakeStore:
    def __init__(self):
        self.values = {
            "debug_server/rank0": b"http://node0:43100",
            "torchelastic/role_info/0": b'{"role":"trainer","rank":0}',
            "default_pg/0//cuda//0": b"\x00\x01secret-binary",
        }

    def list_keys(self):
        return list(self.values)

    def multi_get(self, keys):
        return [self.values[key] for key in keys]


class CountOnlyStore:
    host = "store.internal"
    port = 29400

    def num_keys(self):
        return 17

    def check(self, keys):
        return False


class KnownKeyStore(CountOnlyStore):
    def __init__(self):
        self.values = {
            "torch.rendezvous.job-42": b"opaque-state",
            "torchelastic/role_info/0": b'{"role":"trainer"}',
            "probing/torchrun/job-42/master": b'{"http_base":"http://node0:9922"}',
        }

    def check(self, keys):
        return all(key in self.values for key in keys)

    def multi_get(self, keys):
        return [self.values[key] for key in keys]


def test_normalize_wait_counters_adds_category_and_average():
    rows = runtime_debug._normalize_wait_counters(
        {
            "pytorch.wait_counter.TCPStore__check": {
                "active_count": 0,
                "total_calls": 4,
                "total_time_us": 20,
                "max_time_us": 9,
            }
        },
        rank=3,
    )

    assert rows == [
        {
            "name": "pytorch.wait_counter.TCPStore__check",
            "category": "tcpstore",
            "rank": 3,
            "active_count": 0,
            "total_calls": 4,
            "total_time_us": 20,
            "max_time_us": 9,
            "avg_time_us": 5.0,
        }
    ]


def test_registered_wait_counter_provider_fills_missing_pytorch_handler(monkeypatch):
    def unavailable():
        raise ModuleNotFoundError("torch.distributed.debug")

    def provider():
        return {
            "pytorch.wait_counter.fixture.ProcessGroupDP__reduce_scatter": {
                "active_count": 0,
                "total_calls": 3,
                "total_time_us": 90,
                "max_time_us": 40,
            }
        }

    monkeypatch.setattr(runtime_debug, "_snapshot_pytorch_wait_counters", unavailable)
    runtime_debug.register_wait_counter_provider(provider, source="test fixture")
    try:
        result = runtime_debug.snapshot_wait_counters()
    finally:
        runtime_debug.unregister_wait_counter_provider(provider)

    assert result["available"] is True
    assert result["source"] == "test fixture"
    assert result["counters"][0]["total_calls"] == 3
    assert result["counters"][0]["avg_time_us"] == 30.0


def test_tcpstore_snapshot_only_previews_known_safe_namespaces():
    result = runtime_debug._snapshot_tcpstore(FakeStore(), include_values=False)

    assert result[0]["key"] == "debug_server/rank0"
    assert result[0]["category"] == "debug_worker"
    assert result[0]["value_preview"] == "http://node0:43100"
    assert result[0]["redacted"] is False

    process_group = next(row for row in result if row["category"] == "process_group")
    assert process_group["value_preview"] == ""
    assert process_group["redacted"] is True
    assert process_group["value_size"] == len(b"\x00\x01secret-binary")


def test_tcpstore_snapshot_can_include_unknown_values_when_explicitly_enabled():
    result = runtime_debug._snapshot_tcpstore(FakeStore(), include_values=True)
    process_group = next(row for row in result if row["category"] == "process_group")

    assert "secret-binary" in process_group["value_preview"]
    assert process_group["redacted"] is False


def test_runtime_snapshot_reports_each_capability_independently(monkeypatch):
    monkeypatch.setattr(
        runtime_debug,
        "snapshot_wait_counters",
        lambda: {"available": False, "error": "unsupported", "counters": []},
    )
    monkeypatch.setattr(
        runtime_debug,
        "snapshot_tcpstore",
        lambda include_values=False: {
            "available": True,
            "error": None,
            "entries": [{"key": "debug_server/rank0"}],
        },
    )

    result = runtime_debug.snapshot_runtime_debug()

    assert result["wait_counters"]["available"] is False
    assert result["tcpstore"]["available"] is True


def test_tcpstore_unknown_values_require_environment_opt_in(monkeypatch):
    monkeypatch.setattr(runtime_debug, "_tcpstore_client", FakeStore)
    monkeypatch.delenv("PROBING_TCPSTORE_INSPECT", raising=False)

    hidden = runtime_debug.snapshot_tcpstore(include_values=True)
    process_group = next(
        row for row in hidden["entries"] if row["category"] == "process_group"
    )
    assert hidden["values_enabled"] is False
    assert process_group["redacted"] is True

    monkeypatch.setenv("PROBING_TCPSTORE_INSPECT", "1")
    visible = runtime_debug.snapshot_tcpstore(include_values=True)
    process_group = next(
        row for row in visible["entries"] if row["category"] == "process_group"
    )
    assert visible["values_enabled"] is True
    assert process_group["redacted"] is False


def test_tcpstore_reports_key_count_when_runtime_cannot_list_keys(monkeypatch):
    monkeypatch.setattr(runtime_debug, "_tcpstore_client", CountOnlyStore)
    monkeypatch.setenv("RANK", "3")
    monkeypatch.setenv("WORLD_SIZE", "64")
    monkeypatch.setenv("LOCAL_RANK", "3")
    monkeypatch.setenv("LOCAL_WORLD_SIZE", "8")
    monkeypatch.setenv("GROUP_RANK", "0")
    monkeypatch.setenv("GROUP_WORLD_SIZE", "8")
    monkeypatch.setenv("ROLE_NAME", "trainer")
    monkeypatch.setenv("ROLE_RANK", "3")
    monkeypatch.setenv("ROLE_WORLD_SIZE", "64")
    monkeypatch.setenv("TORCHELASTIC_RUN_ID", "job-42")
    monkeypatch.setenv("TORCHELASTIC_RESTART_COUNT", "1")
    monkeypatch.setenv("TORCHELASTIC_MAX_RESTARTS", "3")

    result = runtime_debug.snapshot_tcpstore()

    assert result["available"] is True
    assert result["catalog_available"] is False
    assert result["total_keys"] == 17
    assert result["entries"] == []
    assert result["identified_keys"] == 0
    assert result["catalog_mode"] == "known_keys"
    assert result["facts"] == [
        {"label": "Store", "value": "CountOnlyStore · store.internal:29400"},
        {"label": "Run", "value": "job-42"},
        {"label": "Rank", "value": "3 / 64 · local 3 / 8"},
        {"label": "Node", "value": "0 / 8"},
        {"label": "Role", "value": "trainer · 3 / 64"},
        {"label": "Restart", "value": "1 / 3"},
    ]


def test_tcpstore_probes_known_keys_without_native_catalog(monkeypatch):
    monkeypatch.setattr(runtime_debug, "_tcpstore_client", KnownKeyStore)
    monkeypatch.setenv("TORCHELASTIC_RUN_ID", "job-42")
    monkeypatch.setenv("GROUP_WORLD_SIZE", "1")

    result = runtime_debug.snapshot_tcpstore()

    assert result["catalog_available"] is False
    assert result["catalog_mode"] == "known_keys"
    assert result["identified_keys"] == 3
    assert [entry["key"] for entry in result["entries"]] == [
        "probing/torchrun/job-42/master",
        "torch.rendezvous.job-42",
        "torchelastic/role_info/0",
    ]
    rendezvous = next(
        entry for entry in result["entries"] if entry["category"] == "rendezvous"
    )
    assert rendezvous["redacted"] is True
    role = next(entry for entry in result["entries"] if entry["category"] == "role")
    assert role["value_preview"] == '{"role":"trainer"}'

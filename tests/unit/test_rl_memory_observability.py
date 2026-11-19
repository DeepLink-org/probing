"""Unit tests for the RL memory-observability additions."""

from __future__ import annotations

from types import SimpleNamespace


def test_worker_runtime_env_strips_driver_port(monkeypatch):
    monkeypatch.setenv("PROBING", "1")
    monkeypatch.setenv("PROBING_PORT", "18081")
    monkeypatch.setenv("PROBING_ORIGINAL", "1")

    from probing.ext.ray import worker_runtime_env

    env = worker_runtime_env()
    assert env["PROBING"] == "1"
    assert "PROBING_PORT" not in env
    assert "PROBING_ORIGINAL" not in env
    assert env["PROBING_GPU_SAMPLE_MS"] == "0"


def test_worker_runtime_env_explicit_extra_env_wins(monkeypatch):
    monkeypatch.setenv("PROBING", "1")

    from probing.ext.ray import worker_runtime_env

    env = worker_runtime_env({"PROBING_GPU_SAMPLE_MS": "60000"})
    assert env["PROBING_GPU_SAMPLE_MS"] == "60000"


def test_ray_store_row_maps_driver_stats():
    from probing.ext.ray import RAY_STORE_FIELDS, _store_row

    stats = SimpleNamespace(
        object_store_bytes_used=1,
        object_store_bytes_avail=2,
        spilled_bytes_total=3,
        spilled_objects_total=4,
        restored_bytes_total=5,
        num_local_objects=6,
        num_object_store_primary_copies=7,
        object_pulls_queued=8,
        cumulative_created_bytes=9,
    )
    row = _store_row(100, "host", "node", stats)
    assert len(row) == len(RAY_STORE_FIELDS)
    assert row == [100, "host", "node", 1, 2, 3, 4, 5, 6, 7, 8, 9]

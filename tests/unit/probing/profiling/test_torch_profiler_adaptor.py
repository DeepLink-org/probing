"""Unit tests for torch_profiler Kineto → hotspot adaptor."""

from __future__ import annotations

from dataclasses import dataclass
from unittest.mock import MagicMock

import pytest

from probing.profiling.torch_profiler.adaptor import (
    _bucket_kind_for_name,
    compile_from_profiler,
    compile_key_averages,
)
from probing.profiling.torch_profiler.session_store import (
    CaptureRecord,
    HotspotRecord,
    SessionStore,
)


@dataclass
class _FakeEvent:
    key: str
    self_cuda_time_total: int = 0
    self_cpu_time_total: int = 0
    cuda_time_total: int = 0
    cpu_time_total: int = 0
    count: int = 1


@pytest.fixture
def stub_coords(monkeypatch):
    monkeypatch.setattr(
        "probing.profiling.torch_profiler.adaptor.row_fields",
        lambda _snap=None: {
            "local_step": 7,
            "global_step": 7,
            "rank": 2,
            "world_size": 8,
        },
    )
    monkeypatch.setattr(
        "probing.profiling.torch_profiler.adaptor.current_role",
        lambda: "dp=2",
    )


@pytest.mark.parametrize(
    ("name", "expected"),
    [
        ("nccl:all_reduce", "collective"),
        ("Memcpy HtoD", "memcpy"),
        ("cudaDeviceSynchronize", "cuda_runtime"),
        ("cudaLaunchKernel", "cuda_runtime"),
        ("aten::mm", "cpu_op"),
        ("autograd::engine", "cpu_op"),
        ("void at::native::vectorized_elementwise_kernel", "kernel"),
    ],
)
def test_bucket_kind_mapping(name, expected):
    assert _bucket_kind_for_name(name) == expected


def test_compile_key_averages_buckets_and_pct(stub_coords):
    events = [
        _FakeEvent("nccl:all_reduce", self_cuda_time_total=300, cuda_time_total=400),
        _FakeEvent("aten::mm", self_cpu_time_total=100, cpu_time_total=150),
        _FakeEvent("Memcpy HtoD (Pinned -> Device)", self_cuda_time_total=50),
    ]
    capture, hotspots = compile_key_averages(
        events,
        trigger="test",
        steps_profiled=1,
        started_at_us=1_000_000,
        ended_at_us=1_500_000,
    )
    assert capture.status == "completed"
    assert capture.local_step == 7
    assert capture.rank == 2
    assert capture.wall_us == 500_000
    kinds = {h.bucket_kind for h in hotspots}
    assert kinds == {"collective", "cpu_op", "memcpy"}
    total_pct = sum(h.pct_of_capture for h in hotspots)
    assert abs(total_pct - 450 / 500_000) < 1e-6
    top = max(hotspots, key=lambda h: h.self_us)
    assert top.bucket_kind == "collective"


def test_compile_key_averages_merges_duplicate_buckets(stub_coords):
    events = [
        _FakeEvent("aten::mm", self_cpu_time_total=40, count=2),
        _FakeEvent("aten::mm", self_cpu_time_total=60, count=3),
    ]
    _, hotspots = compile_key_averages(
        events,
        trigger="test",
        steps_profiled=1,
        started_at_us=0,
        ended_at_us=100,
    )
    assert len(hotspots) == 1
    assert hotspots[0].self_us == 100
    assert hotspots[0].calls == 5


def test_compile_key_averages_truncation(monkeypatch, stub_coords):
    monkeypatch.setattr(
        "probing.profiling.torch_profiler.adaptor._max_events",
        lambda: 2,
    )
    events = [_FakeEvent(f"op{i}", self_cpu_time_total=10) for i in range(5)]
    capture, hotspots = compile_key_averages(
        events,
        trigger="test",
        steps_profiled=1,
        started_at_us=0,
        ended_at_us=100,
    )
    assert capture.truncated is True
    assert capture.event_count == 5
    assert len(hotspots) == 2


def test_compile_from_profiler_uses_key_averages(stub_coords):
    profiler = MagicMock()
    profiler.key_averages.return_value = [
        _FakeEvent("aten::add", self_cpu_time_total=25),
    ]
    capture, hotspots = compile_from_profiler(
        profiler,
        trigger="unit",
        steps_profiled=1,
        started_at_us=0,
        ended_at_us=50,
    )
    assert capture.trigger == "unit"
    assert len(hotspots) == 1
    assert hotspots[0].bucket_name == "aten::add"


def test_compile_from_profiler_falls_back_to_events(stub_coords):
    profiler = MagicMock()
    profiler.key_averages.side_effect = RuntimeError("not ready")
    profiler.events.return_value = [_FakeEvent("kernel_a", self_cuda_time_total=10)]
    capture, hotspots = compile_from_profiler(
        profiler,
        trigger="unit",
        steps_profiled=1,
        started_at_us=0,
        ended_at_us=50,
    )
    assert capture.status == "completed"
    assert capture.error == ""
    assert len(hotspots) == 1
    assert hotspots[0].bucket_name == "kernel_a"


def test_session_store_bounded(monkeypatch):
    monkeypatch.setenv("PROBING_TORCH_PROFILER_MAX_SESSIONS", "2")
    store = SessionStore(max_sessions=2)
    for i in range(3):
        cid = f"c{i}"
        store.add_capture(
            CaptureRecord(capture_id=cid, status="completed"),
            [HotspotRecord(capture_id=cid, bucket_name="k", self_us=1)],
        )
    assert len(store.captures()) == 2
    assert store.captures()[0].capture_id == "c1"
    assert all(h.capture_id != "c0" for h in store.hotspots())

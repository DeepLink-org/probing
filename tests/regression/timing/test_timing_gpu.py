"""End-to-end timing tests that run on a real CUDA device.

These drive the actual stream-value-gated CUDA-event path. They are skipped when
no CUDA device is present (e.g. CPU-only CI) and skipped gracefully when the
device lacks driver stream memory operations.
"""

from __future__ import annotations

import pytest

torch = pytest.importorskip("torch")

pytestmark = [
    pytest.mark.integration,
    pytest.mark.slow,
    pytest.mark.skipif(
        not torch.cuda.is_available(), reason="requires a CUDA device"
    ),
]

from probing.timing import TimingRecord, timing, timing_context  # noqa: E402

_DEVICE = 0
_METHOD = "cuda_event_wait_value32_ffi"


def _warm_up(fn) -> None:
    """Run ``fn`` once outside any gate and sync.

    The stream-value gate holds the compute stream behind a device-side flag, so
    a host-side sync the workload triggers on its first call (lazy cuBLAS init,
    workspace allocation) would deadlock behind the still-closed gate. Real
    profiling warms up the same way before measuring.
    """
    fn()
    torch.cuda.synchronize(_DEVICE)


def test_timing_context_measures_real_workload():
    a = torch.randn(512, 512, device=f"cuda:{_DEVICE}")
    b = torch.randn(512, 512, device=f"cuda:{_DEVICE}")

    @timing
    def gemm():
        return a @ b

    _warm_up(gemm)

    try:
        with timing_context(device=_DEVICE) as ctx:
            out = gemm()
    except RuntimeError as exc:  # pragma: no cover - driver dependent
        pytest.skip(f"stream-value gate unavailable on cuda:{_DEVICE}: {exc}")

    assert out.shape == (512, 512)
    assert ctx["gemm"] > 0.0
    assert ctx.methods["gemm"] == _METHOD
    rec = ctx.records["gemm"]
    assert isinstance(rec, TimingRecord)
    assert rec.value is out


def test_timing_decorator_is_noop_outside_context():
    a = torch.randn(64, 64, device=f"cuda:{_DEVICE}")

    @timing
    def gemm():
        return a @ a

    out = gemm()  # no active context -> runs untouched, nothing recorded
    assert out.shape == (64, 64)


def test_timing_context_reuses_gate_across_iterations():
    a = torch.randn(256, 256, device=f"cuda:{_DEVICE}")

    @timing
    def relu():
        return torch.relu(a)

    _warm_up(relu)

    elapsed = []
    for _ in range(5):
        try:
            with timing_context(device=_DEVICE) as ctx:
                relu()
        except RuntimeError as exc:  # pragma: no cover - driver dependent
            pytest.skip(f"stream-value gate unavailable on cuda:{_DEVICE}: {exc}")
        elapsed.append(ctx["relu"])

    # Records are keyed by workload name, so repeated calls overwrite in place.
    assert len(elapsed) == 5
    assert all(ms > 0.0 for ms in elapsed)
    assert ctx.methods["relu"] == _METHOD

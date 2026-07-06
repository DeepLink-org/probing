"""Crash hook Flight Recorder autostart tests."""

from probing import crash as crash_mod


def test_watchdog_detection():
    assert crash_mod._looks_like_nccl_watchdog(
        RuntimeError(
            "Watchdog caught collective operation timeout: WorkNCCL(SeqNum=1, OpType=ALLREDUCE)"
        )
    )
    assert not crash_mod._looks_like_nccl_watchdog(RuntimeError("something else"))


def test_flight_recorder_snapshot_on_watchdog(monkeypatch):
    monkeypatch.setenv("PROBING_FR_ON_WATCHDOG", "on")
    calls: list[dict] = []

    def fake_snapshot(**kwargs):
        calls.append(kwargs)
        return {"ok": True}

    monkeypatch.setattr(
        "probing.profiling.flight_recorder.snapshot",
        fake_snapshot,
    )
    crash_mod._maybe_snapshot_flight_recorder(
        RuntimeError("Watchdog caught collective operation timeout: WorkNCCL")
    )
    assert calls == [{"only_active": True, "persist": True}]

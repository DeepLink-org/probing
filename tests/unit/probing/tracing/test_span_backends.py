"""Span multi-backend recorder tests."""

from __future__ import annotations

import dataclasses

import pytest

import probing


@pytest.fixture(autouse=True)
def _isolate_trace_table(monkeypatch):
    from probing.tracing import TraceEvent, bind_table, reset_backends

    monkeypatch.delenv("PROBING_SPAN_BACKENDS", raising=False)
    try:
        TraceEvent.drop()
    except Exception:
        pass
    TraceEvent.init_table()
    reset_backends(clear_registered=True)
    bind_table(TraceEvent)
    yield
    reset_backends(clear_registered=True)


def _trace_rows(n: int = 50) -> list[dict]:
    from probing.tracing import TraceEvent

    fields = [f.name for f in dataclasses.fields(TraceEvent)]
    return [dict(zip(fields, data)) for _ts, data in TraceEvent.take(n)]


def test_default_backend_is_memtable():
    from probing.tracing import list_backends

    assert list_backends() == ["memtable"]


def test_custom_backend_receives_span_lifecycle(monkeypatch):
    from probing.tracing import register_backend, reset_backends

    calls: list[tuple[str, object]] = []

    class CaptureBackend:
        name = "capture"

        def on_span_start(self, record):
            calls.append(("start", record.name))

        def on_span_end(self, record):
            calls.append(("end", record.span_id))

        def on_event(self, record):
            calls.append(("event", record.name))

        def shutdown(self):
            calls.append(("shutdown", None))

    register_backend("capture", lambda: CaptureBackend())
    monkeypatch.setenv("PROBING_SPAN_BACKENDS", "memtable,capture")
    reset_backends()

    with probing.span("dual") as span:
        span_id = span.span_id
        probing.event("ping")

    assert ("start", "dual") in calls
    assert ("event", "ping") in calls
    assert any(c[0] == "end" and c[1] == span_id for c in calls)

    rows = _trace_rows()
    assert any(
        r.get("record_type") == "span_start" and r.get("name") == "dual" for r in rows
    )


def test_unknown_backend_falls_back_to_memtable_only(monkeypatch):
    from probing.tracing import list_backends, reset_backends

    monkeypatch.setenv("PROBING_SPAN_BACKENDS", "unknown_backend")
    reset_backends()
    assert list_backends() == ["memtable"]

    with probing.span("still_works"):
        pass

    rows = _trace_rows()
    assert any(r.get("name") == "still_works" for r in rows)


def test_configure_overrides_env(monkeypatch):
    from probing.tracing import configure_backends, list_backends

    monkeypatch.setenv("PROBING_SPAN_BACKENDS", "unknown_backend")
    configure_backends(["memtable"])
    assert list_backends() == ["memtable"]


def test_otel_backend_skipped_without_sdk(monkeypatch):
    from probing.tracing import list_backends, reset_backends

    monkeypatch.setenv("PROBING_SPAN_BACKENDS", "memtable,otel")
    reset_backends()
    assert list_backends() == ["memtable"]


def test_logger_backend_with_memtable(monkeypatch, capsys):
    import logging

    from probing.tracing import list_backends, reset_backends

    log = logging.getLogger("probing.span")
    log.handlers.clear()
    log.propagate = True

    monkeypatch.setenv("PROBING_SPAN_BACKENDS", "memtable,logger")
    reset_backends()
    assert list_backends() == ["memtable", "logger"]

    with probing.span("hello", phase="forward"):
        probing.event("ping", attributes=[{"x": 1}])

    err = capsys.readouterr().err
    assert "→ hello phase=forward" in err
    assert "· ping" in err
    assert "← hello" in err and "ms" in err

    rows = _trace_rows()
    assert any(
        r.get("record_type") == "span_start" and r.get("name") == "hello" for r in rows
    )


def test_logger_backend_only(monkeypatch, capsys):
    import logging

    from probing.tracing import list_backends, reset_backends

    log = logging.getLogger("probing.span")
    log.handlers.clear()
    log.propagate = True

    monkeypatch.setenv("PROBING_SPAN_BACKENDS", "logger")
    reset_backends()
    assert list_backends() == ["logger"]

    with probing.span("terminal_only"):
        pass

    assert "→ terminal_only" in capsys.readouterr().err
    assert _trace_rows() == []

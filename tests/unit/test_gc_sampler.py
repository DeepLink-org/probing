"""GC sampler contract tests (no engine required)."""

from __future__ import annotations

from probing import gc as gc_sampler


def test_gc_sampler_is_disabled_by_env(monkeypatch):
    monkeypatch.setenv("PROBING_GC", "0")
    assert gc_sampler._enabled() is False
    assert gc_sampler.start() is False


def test_gc_sampler_fields_are_stable():
    assert gc_sampler.FIELDS[0] == "ts"
    assert "max_pause_ms" in gc_sampler.FIELDS
    assert "gen2_collections" in gc_sampler.FIELDS

"""Regression guards for TorchProbe overhead semantics (see docs/design/overhead-invariants.*)."""

from __future__ import annotations

import inspect
from pathlib import Path

import pytest

from probing.profiling import deferred_drain as dd
from probing.profiling import torch_probe as tp
from probing.profiling.torch_probe import TorchProbe


def test_close_step_wall_source_order():
    """I3: timing must be recorded before deferred drain (source-level guard)."""
    src = inspect.getsource(TorchProbe._close_step_wall)
    timing_pos = src.index("_record_step_timing")
    drain_pos = src.index("_drain_deferred")
    mark_pos = src.index("_mark_step_wall_start")
    assert timing_pos < drain_pos < mark_pos, (
        "_close_step_wall must be: record → drain → advance → mark "
        "(see docs/src/design/overhead-invariants.zh.md §I3)"
    )


def test_post_step_hook_does_not_drain_before_close_step_wall():
    """I3: post_step_hook must not call _drain_deferred before _close_step_wall."""
    src = inspect.getsource(TorchProbe.post_step_hook)
    assert "_drain_deferred()" not in src.split("_close_step_wall", 1)[0], (
        "post_step_hook must not drain before _close_step_wall"
    )


def test_deferred_drain_async_default_on(monkeypatch):
    """I4: async deferred drain is on by default (conftest forces sync for other tests)."""
    monkeypatch.delenv("PROBING_TORCH_DEFER_ASYNC", raising=False)
    assert dd.deferred_drain_async_enabled() is True


def test_overhead_invariants_doc_exists():
    """Design doc SSOT is present for agents."""
    repo = Path(__file__).resolve().parents[3]
    zh = repo / "docs/src/design/overhead-invariants.zh.md"
    en = repo / "docs/src/design/overhead-invariants.md"
    assert zh.is_file(), f"missing {zh}"
    assert en.is_file(), f"missing {en}"
    text = zh.read_text(encoding="utf-8")
    for needle in (
        "median(dispatch)",
        "(1 − rate)",
        "_record_step_timing",
        "PROBING_TORCH_DEFER_ASYNC",
    ):
        assert needle in text, f"overhead-invariants doc missing: {needle!r}"


def test_defer_settle_constants_documented_range():
    """Defer settle window is part of overhead stability contract."""
    assert tp._DEFER_MIN_SETTLE >= 1
    assert tp._DEFER_MIN_SETTLE < tp._DEFER_MAX_LAG

"""Tests for playbook loader."""

from __future__ import annotations

import pytest

yaml = pytest.importorskip("yaml")

from probing.playbooks.loader import (
    expand_playbook,
    load_catalog,
    load_playbook,
    match_playbooks,
    playbooks_root,
    validate_all,
)


def test_playbooks_root_exists():
    root = playbooks_root()
    assert root.is_dir()
    assert (root / "catalog.yaml").is_file()


def test_catalog_loads_seven_playbooks():
    catalog = load_catalog()
    assert len(catalog.playbooks) == 7
    ids = {p.id for p in catalog.playbooks}
    assert "health_overview" in ids
    assert "slow_rank" in ids


def test_expand_slow_rank_global():
    pb = load_playbook("slow_rank")
    steps = expand_playbook(pb, {"use_global": True, "step_window": 10})
    rank_step = next(s for s in steps if s.id == "rank_latency")
    assert rank_step.sql is not None
    assert "global.python.comm_collective" in rank_step.sql
    assert "{step_window}" not in rank_step.sql


def test_expand_slow_rank_local():
    pb = load_playbook("slow_rank")
    steps = expand_playbook(pb, {"use_global": False, "step_window": 5})
    rank_step = next(s for s in steps if s.id == "rank_latency")
    assert "python.comm_collective" in rank_step.sql
    assert "global." not in rank_step.sql


def test_match_playbooks_hang():
    matched = match_playbooks("训练卡住了 hang")
    assert "training_hang" in matched


def test_match_playbooks_straggler():
    matched = match_playbooks("哪个 rank 拖后腿 straggler")
    assert "slow_rank" in matched


def test_validate_all_clean():
    warnings = validate_all()
    assert warnings == []

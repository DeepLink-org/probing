"""Tests for skill loader."""

from __future__ import annotations

import pytest

pytest.importorskip("yaml")

from probing.skills.loader import (
    expand_skill,
    load_catalog,
    load_skill,
    match_skills,
    skills_root,
)


def test_skills_root_exists():
    from probing.skills.paths import repo_skills_dir

    repo = repo_skills_dir()
    assert repo is not None
    assert repo.is_dir()
    assert (repo / "catalog.yaml").is_file()
    root = skills_root()
    assert root.is_dir()


def test_catalog_loads_eight_skills():
    catalog = load_catalog()
    assert len(catalog.skills) == 8
    ids = {p.id for p in catalog.skills}
    assert "slow_rank" in ids
    assert "nccl_culprit_victim" in ids
    assert "health_overview" in ids


def test_load_slow_rank_global():
    skill = load_skill("slow_rank")
    steps = expand_skill(skill, {"use_global": True, "step_window": 10})
    assert steps
    sql = " ".join(s.sql or "" for s in steps if s.sql)
    assert "global." in sql


def test_load_slow_rank_local():
    skill = load_skill("slow_rank")
    steps = expand_skill(skill, {"use_global": False, "step_window": 5})
    rank_latency = next(s for s in steps if s.id == "rank_latency")
    assert rank_latency.sql is not None
    assert "global.python.comm_collective" not in rank_latency.sql
    assert "python.comm_collective" in rank_latency.sql


def test_match_skills_hang():
    matched = match_skills("训练卡住了 hang")
    assert "training_hang" in matched


def test_match_skills_straggler():
    matched = match_skills("哪个 rank 拖后腿 straggler")
    assert "slow_rank" in matched

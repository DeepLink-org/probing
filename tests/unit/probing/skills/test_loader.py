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


def test_catalog_loads_nine_skills():
    catalog = load_catalog()
    assert len(catalog.skills) == 9
    ids = {p.id for p in catalog.skills}
    assert "crash_triage" in ids
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


def _normalize_sql(sql: str) -> str:
    return " ".join(sql.split())


def test_slow_rank_rank_latency_sql_golden_parity():
    """Rust cli/skill/loader.rs tests must produce the same expanded SQL."""
    skill = load_skill("slow_rank")
    steps = expand_skill(skill, {"use_global": False, "step_window": 5})
    rank_latency = next(s for s in steps if s.id == "rank_latency")
    normalized = _normalize_sql(rank_latency.sql or "")
    assert "FROM python.comm_collective" in normalized
    assert "global.python.comm_collective" not in normalized
    assert "- 5" in normalized

    steps_global = expand_skill(skill, {"use_global": True, "step_window": 10})
    rank_latency_global = next(s for s in steps_global if s.id == "rank_latency")
    normalized_global = _normalize_sql(rank_latency_global.sql or "")
    assert "FROM global.python.comm_collective" in normalized_global
    assert "- 10" in normalized_global


def test_match_skills_hang():
    matched = match_skills("训练卡住了 hang")
    assert "training_hang" in matched


def test_match_skills_straggler():
    matched = match_skills("哪个 rank 拖后腿 straggler")
    assert "slow_rank" in matched

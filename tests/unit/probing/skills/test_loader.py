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


def test_catalog_loads_all_skills():
    catalog = load_catalog()
    expected_ids = {
        "health_overview",
        "job_health",
        "crash_triage",
        "training_hang",
        "slow_rank",
        "persistent_straggler",
        "memory_leak",
        "module_bottleneck",
        "comm_bottleneck",
        "gpu_pressure",
        "nccl_culprit_victim",
        "watchdog_timeout",
        "sre_triage",
    }
    ids = {p.id for p in catalog.skills}
    assert ids == expected_ids
    assert len(catalog.skills) == len(expected_ids)


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


def test_comm_bottleneck_nccl_coll_perf_expansion():
    """Precise NCCL layer must expand independently of the legacy comm table."""
    skill = load_skill("comm_bottleneck")
    local_steps = expand_skill(skill, {"use_global": False})
    coll_bw = next(s for s in local_steps if s.id == "nccl_coll_bw")
    assert coll_bw.sql is not None
    assert "FROM nccl.coll_perf" in coll_bw.sql
    assert "timing_source" in coll_bw.sql
    assert "comm_collective" not in coll_bw.sql

    global_steps = expand_skill(skill, {"use_global": True})
    coll_bw_global = next(s for s in global_steps if s.id == "nccl_coll_bw")
    assert "FROM global.nccl.coll_perf" in (coll_bw_global.sql or "")


def test_match_skills_hang():
    matched = match_skills("训练卡住了 hang")
    assert "training_hang" in matched


def test_match_skills_straggler():
    matched = match_skills("哪个 rank 拖后腿 straggler")
    assert "slow_rank" in matched


def test_match_skills_watchdog_timeout():
    matched = match_skills("NCCL watchdog timeout flight recorder")
    assert "watchdog_timeout" in matched


def test_watchdog_timeout_table_expansion():
    skill = load_skill("watchdog_timeout")
    local_steps = expand_skill(skill, {"use_global": False, "seq_window": 7})
    local_sql = " ".join(s.sql or "" for s in local_steps)
    assert "FROM python.torch_nccl_flight_record" in local_sql
    assert "global.python.torch_nccl_flight_record" not in local_sql
    assert "- 7" in local_sql

    global_steps = expand_skill(skill, {"use_global": True, "seq_window": 11})
    global_sql = " ".join(s.sql or "" for s in global_steps)
    assert "FROM global.python.torch_nccl_flight_record" in global_sql
    assert "- 11" in global_sql


def test_match_skills_sre_triage():
    matched = match_skills("SRE 值班 runbook 事故第一响应")
    assert "sre_triage" in matched

"""NCCL culprit/victim skill: loader expansion + mock SQL integration."""

from __future__ import annotations

import os

import pytest

pytest.importorskip("yaml")

pytestmark = pytest.mark.skipif(
    os.environ.get("PROBING") not in ("1", "2", "followed", "nested"),
    reason="needs in-process probing engine (PROBING=1)",
)


@pytest.fixture(autouse=True)
def _enable_nccl_mock(monkeypatch):
    monkeypatch.setenv("PROBING_NCCL_MOCK", "1")


def _require_nccl_proxy_table():
    from probing.nccl.mock import seed_mock
    from probing import query

    seed_mock(ranks=8, ops_per_rank=3)
    for name in ("nccl.proxy_ops", "python.nccl.proxy_ops"):
        try:
            df = query(f"SELECT * FROM {name} LIMIT 1")
            if len(df.columns) > 0:
                return name
        except Exception:
            continue
    pytest.skip("nccl.proxy_ops not in SQL catalog — run `make develop`")


def test_catalog_includes_nccl_skill():
    from probing.skills.loader import load_catalog

    catalog = load_catalog()
    ids = {p.id for p in catalog.skills}
    assert "nccl_culprit_victim" in ids


def test_expand_nccl_skill_global():
    from probing.skills.loader import expand_skill, load_skill

    skill = load_skill("nccl_culprit_victim")
    steps = expand_skill(skill, {"use_global": True, "seq_window": 5})
    sql = " ".join(s.sql or "" for s in steps if s.sql)
    assert "global.nccl.proxy_ops" in sql
    assert "send_gpu_wait_ns" in sql
    assert "recv_wait_ns" in sql


def test_match_skills_nccl_keywords():
    from probing.skills.loader import match_skills

    matched = match_skills("NCCL culprit recv_wait 受害者")
    assert "nccl_culprit_victim" in matched


def test_skill_sql_on_mock_data():
    from probing.nccl.mock import _CULPRIT_RANK, _VICTIM_RANK
    from probing import query
    from probing.skills.interpret import evaluate_rules, evidence_from_dataframe
    from probing.skills.loader import expand_skill, load_skill

    table = _require_nccl_proxy_table()

    skill = load_skill("nccl_culprit_victim")
    steps = expand_skill(skill, {"use_global": False, "seq_window": 10})

    rank_step = next(s for s in steps if s.id == "rank_wait_summary")
    sql = (
        rank_step.sql.replace("nccl.proxy_ops", table)
        if table != "nccl.proxy_ops"
        else rank_step.sql
    )
    df = query(sql)
    assert len(df) >= 2
    assert "total_gpu_wait_ns" in df.columns
    assert "total_recv_wait_ns" in df.columns

    culprit_df = query(
        f"""
        SELECT rank, sum(send_gpu_wait_ns) AS culprit_wait_ns
        FROM {table}
        GROUP BY rank
        ORDER BY culprit_wait_ns DESC
        LIMIT 1
        """
    )
    victim_df = query(
        f"""
        SELECT rank, sum(recv_wait_ns) AS victim_wait_ns
        FROM {table}
        GROUP BY rank
        ORDER BY victim_wait_ns DESC
        LIMIT 1
        """
    )
    assert culprit_df["rank"].iloc[0] == _CULPRIT_RANK
    assert victim_df["rank"].iloc[0] == _VICTIM_RANK

    rules = skill.interpretation.get("rules") or []
    evidence = [
        evidence_from_dataframe(
            "rank_wait_summary",
            df[["total_gpu_wait_ns", "total_recv_wait_ns"]].to_dict("list"),
        )
    ]
    fired = evaluate_rules(rules, evidence)
    assert any(r.rule_id in ("culprit_dominates", "victim_dominates") for r in fired)

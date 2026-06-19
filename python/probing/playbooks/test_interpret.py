"""Tests for playbook interpretation rules."""

from probing.playbooks.interpret import (
    StepEvidence,
    evaluate_rules,
    evidence_from_dataframe,
    rule_matches,
)


def test_rows_zero():
    steps = [StepEvidence("available_tables", 0)]
    rules = [
        {
            "id": "no_tables",
            "when": "step:available_tables | rows == 0",
            "severity": "error",
            "message": "no tables",
        }
    ]
    assert len(evaluate_rules(rules, steps)) == 1


def test_max_min_ratio():
    steps = [
        evidence_from_dataframe(
            "rank_latency",
            {"avg_ms": [10.0, 20.0, 40.0]},
        )
    ]
    rules = [
        {
            "id": "straggler",
            "when": "step:rank_latency | column:avg_ms | max/min(ratio) > 1.5",
            "severity": "warning",
            "message": "slow",
        }
    ]
    assert len(evaluate_rules(rules, steps)) == 1


def test_param_expansion_in_rows():
    steps = [StepEvidence("monotonic_growth_steps", 8)]
    assert rule_matches(
        "step:monotonic_growth_steps | rows > {min_steps} * 0.7",
        steps,
        {"min_steps": "10"},
    )

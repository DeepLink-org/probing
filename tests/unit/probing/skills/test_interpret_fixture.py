"""Contract tests for shared skill fixtures (Rust is SSOT for evaluation)."""

from __future__ import annotations

from pathlib import Path

import pytest

pytest.importorskip("yaml")

import yaml

FIXTURES = Path(__file__).resolve().parents[3] / "fixtures"


def test_interpret_parity_fixture_has_expected_cases():
    raw = (FIXTURES / "skill_interpret_parity.yaml").read_text(encoding="utf-8")
    doc = yaml.safe_load(raw)
    names = {c["name"] for c in doc["cases"]}
    assert names == {
        "rows_zero",
        "max_min_ratio",
        "param_rows_threshold",
        "value_eq_zero",
        "ratio_slow_steps",
    }
    for case in doc["cases"]:
        assert case["rules"]
        assert "expect_count" in case


def test_derived_variables_fixture_covers_global_and_local():
    raw = (FIXTURES / "skill_derived_variables.yaml").read_text(encoding="utf-8")
    doc = yaml.safe_load(raw)
    by_name = {c["name"]: c["expected"] for c in doc["cases"]}
    assert set(by_name) == {"local", "global"}
    assert by_name["local"]["comm_table"] == "python.comm_collective"
    assert by_name["global"]["nccl_proxy_table"] == "global.nccl.proxy_ops"
    assert by_name["global"]["global_prefix"] == "global."

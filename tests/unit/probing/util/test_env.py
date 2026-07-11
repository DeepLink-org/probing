"""Unit tests for shared env/config boolean parsing."""

from __future__ import annotations

import pytest

from probing.util.env import FALSE_VALUES, TRUE_VALUES, parse_bool_flag


@pytest.mark.parametrize(
    "value",
    sorted(TRUE_VALUES),
)
def test_parse_bool_flag_true_tokens(value: str) -> None:
    assert parse_bool_flag(value) is True
    assert parse_bool_flag(f"  {value.upper()}  ") is True


@pytest.mark.parametrize(
    "value",
    sorted(FALSE_VALUES),
)
def test_parse_bool_flag_false_tokens(value: str) -> None:
    assert parse_bool_flag(value) is False
    assert parse_bool_flag(f"  {value.upper()}  ") is False


@pytest.mark.parametrize("value", [None, "", "auto", "maybe", "2"])
def test_parse_bool_flag_unknown_or_empty(value: str | None) -> None:
    assert parse_bool_flag(value) is None

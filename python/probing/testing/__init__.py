"""Testing helpers for the probing package.

This module exposes small data factories used in unit tests. Keeping them here
prevents the public probe APIs from pulling in extra helpers as side effects.
"""

from __future__ import annotations

__all__ = [
    "get_dict",
    "get_dict_list",
    "get_list",
    "get_set",
    "get_tuple",
]


def get_dict() -> dict[str, object]:
    return {
        "int": 1,
        "float": 1.0,
        "str": "str",
    }


def get_list() -> list[object]:
    return [
        1,
        1.0,
        "str",
    ]


def get_tuple() -> tuple[object, object, object]:
    return (
        1,
        1.0,
        "str",
    )


def get_set() -> set[object]:
    return {
        1,
        1.0,
        "str",
    }


def get_dict_list() -> list[dict[str, object]]:
    return [
        {
            "int": 1,
            "float": 1.0,
            "str": "str",
        },
        {
            "int": 2,
            "float": 2.0,
            "str": "str2",
        },
    ]

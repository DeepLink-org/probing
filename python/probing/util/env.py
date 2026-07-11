"""Parse common truthy/falsey environment and config strings."""

from __future__ import annotations

from typing import Optional

TRUE_VALUES = frozenset({"1", "true", "yes", "on", "enable", "enabled"})
FALSE_VALUES = frozenset({"0", "false", "no", "off", "disable", "disabled"})


def parse_bool_flag(value: Optional[str]) -> Optional[bool]:
    """Return True/False for known tokens; None when unset or unrecognized."""
    if value is None:
        return None
    normalized = str(value).strip().lower()
    if normalized in TRUE_VALUES:
        return True
    if normalized in FALSE_VALUES:
        return False
    return None

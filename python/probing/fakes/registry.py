"""Fake package registry: prefix → factory."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Callable, Iterable, Optional

import types

ModuleFactory = Callable[[str], types.ModuleType]


@dataclass(frozen=True)
class FakeSpec:
    """One fakeable package family.

    ``prefixes`` are matched with exact equality or ``prefix.`` child paths
    (e.g. ``megatron`` covers ``megatron.core.parallel_state``).
    """

    name: str
    prefixes: tuple[str, ...]
    factory: ModuleFactory
    description: str = ""


_SPECS: dict[str, FakeSpec] = {}
_ENABLED: set[str] = set()


def register(spec: FakeSpec) -> FakeSpec:
    _SPECS[spec.name] = spec
    return spec


def registered() -> dict[str, FakeSpec]:
    return dict(_SPECS)


def enable(names: Iterable[str] | None = None) -> set[str]:
    """Enable specs by name. ``None`` / ``{\"*\", \"all\", \"1\"}`` enables all."""
    global _ENABLED
    if names is None:
        _ENABLED = set(_SPECS)
        return set(_ENABLED)

    normalized = {str(n).strip().lower() for n in names if str(n).strip()}
    if not normalized or normalized & {"*", "all", "1", "true", "on", "yes"}:
        _ENABLED = set(_SPECS)
        return set(_ENABLED)

    aliases = {
        "te": "transformer_engine",
        "flash": "flash_attn",
        "flash-attn": "flash_attn",
    }
    resolved: set[str] = set()
    for name in normalized:
        key = aliases.get(name, name)
        if key not in _SPECS:
            raise KeyError(f"unknown fake spec {name!r}; known: {sorted(_SPECS)}")
        resolved.add(key)
    _ENABLED = resolved
    return set(_ENABLED)


def disable_all() -> None:
    global _ENABLED
    _ENABLED = set()


def enabled_names() -> set[str]:
    return set(_ENABLED)


def match_spec(fullname: str) -> Optional[FakeSpec]:
    for name in _ENABLED:
        spec = _SPECS[name]
        for prefix in spec.prefixes:
            if fullname == prefix or fullname.startswith(prefix + "."):
                return spec
    return None


def parse_fakes_env(raw: str | None) -> Optional[set[str]]:
    """Parse ``PROBING_FAKES``.

    Returns:
      * ``None`` — unset / empty / off → do not install
      * ``set()`` sentinel via empty enable-all handled by caller using ``{\"*\"}``
    """
    if raw is None:
        return None
    text = str(raw).strip()
    if not text:
        return None
    lowered = text.lower()
    if lowered in {"0", "off", "false", "no", "disable", "disabled"}:
        return None
    if lowered in {"1", "true", "on", "yes", "all", "*"}:
        return {"*"}
    return {part.strip() for part in text.split(",") if part.strip()}

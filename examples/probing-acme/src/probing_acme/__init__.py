"""probing-acme — template vendor extension for probing."""

from __future__ import annotations

from pathlib import Path


def skill_root() -> Path:
    """Return the skill tree root (contains catalog.yaml)."""
    return Path(__file__).resolve().parent / "skills"

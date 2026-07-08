"""Shared types for extension discovery."""

from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True)
class SkillRoot:
    """A directory tree that contains skill folders and ``catalog.yaml``."""

    path: Path
    label: str

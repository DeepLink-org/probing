"""JSON serialization helpers for skill HTTP APIs (Rust SSOT).

Kept for backward-compatible imports; handlers should call ``probing._core`` directly.
"""

from __future__ import annotations

import json
from typing import Any, Dict

import probing._core as core

from probing.skills.loader import Skill, SkillCatalog, SkillCatalogEntry, SkillStep


def skill_step_to_dict(step: SkillStep) -> Dict[str, Any]:
    return {
        "id": step.id,
        "title": step.title,
        "type": step.type,
        "sql": step.sql,
        "method": step.method,
        "path": step.path,
        "action": step.action,
        "view": step.view,
        "on_empty": step.on_empty,
        "empty_message": step.empty_message,
        "when": step.when,
        "cluster": step.raw.get("cluster"),
    }


def skill_to_dict(skill: Skill) -> Dict[str, Any]:
    return json.loads(core.skills_load(skill.id))


def catalog_entry_to_dict(entry: SkillCatalogEntry) -> Dict[str, Any]:
    return {
        "id": entry.id,
        "path": entry.path,
        "category": entry.category,
        "priority": entry.priority,
        "description": entry.description,
    }


def catalog_to_dict(catalog: SkillCatalog) -> Dict[str, Any]:
    return {"skills": [catalog_entry_to_dict(e) for e in catalog.skills]}

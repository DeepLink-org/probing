"""Skill discovery helpers — semantics live in Rust (``probing-skills``).

This module keeps lightweight dataclass wrappers and path helpers for Python
tooling. YAML parsing, template expansion, and routing run in Rust via
``probing._core.skills_*``.
"""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Dict, List, Mapping, Optional


def _core():
    import probing._core as core

    return core


@dataclass
class SkillStep:
    id: str
    title: str
    type: str
    sql: Optional[str] = None
    method: Optional[str] = None
    path: Optional[str] = None
    action: Optional[str] = None
    view: Optional[str] = None
    on_empty: str = "skip"
    empty_message: Optional[str] = None
    when: Optional[str] = None
    platform: Optional[str] = None
    raw: Dict[str, Any] = field(default_factory=dict)


@dataclass
class Skill:
    id: str
    title: str
    category: str
    tags: List[str]
    docs: str
    parameters: List[Dict[str, Any]]
    steps: List[SkillStep]
    interpretation: Dict[str, Any]
    summary_template: str
    next_steps: List[str]
    path: Path


@dataclass
class SkillCatalogEntry:
    id: str
    path: str
    category: str
    priority: int
    description: str


@dataclass
class SkillCatalog:
    skills: List[SkillCatalogEntry]


def skills_root() -> Path:
    from probing.skills.paths import skill_roots

    roots = skill_roots()
    if roots:
        return roots[-1].path
    repo = Path(__file__).resolve().parents[3] / "skills"
    return repo


def _step_from_raw(raw: Mapping[str, Any]) -> SkillStep:
    return SkillStep(
        id=str(raw.get("id", "")),
        title=str(raw.get("title", "")),
        type=str(raw.get("type", "sql")),
        sql=raw.get("sql"),
        method=raw.get("method"),
        path=raw.get("path"),
        action=raw.get("action"),
        view=raw.get("view"),
        on_empty=str(raw.get("on_empty", "skip")),
        empty_message=raw.get("empty_message"),
        when=raw.get("when"),
        platform=raw.get("platform"),
        raw=dict(raw),
    )


def load_catalog() -> SkillCatalog:
    data = json.loads(_core().skills_catalog())
    entries = [
        SkillCatalogEntry(
            id=str(item["id"]),
            path=str(item.get("path", "")),
            category=str(item.get("category", "")),
            priority=int(item.get("priority", 0)),
            description=str(item.get("description", "")),
        )
        for item in data.get("skills") or []
    ]
    return SkillCatalog(skills=entries)


def load_skill(skill_id: str) -> Skill:
    raw = json.loads(_core().skills_load(skill_id))
    if "error" in raw:
        raise KeyError(raw["error"])
    steps = [_step_from_raw(s) for s in raw.get("steps") or []]
    return Skill(
        id=str(raw["id"]),
        title=str(raw.get("title", raw["id"])),
        category=str(raw.get("category", "general")),
        tags=list(raw.get("tags") or []),
        docs=str(raw.get("docs") or "").strip(),
        parameters=list(raw.get("parameters") or []),
        steps=steps,
        interpretation=dict(raw.get("interpretation") or {}),
        summary_template=str(raw.get("summary_template") or "").strip(),
        next_steps=list(raw.get("next_steps") or []),
        path=skills_root() / skill_id / "steps.yaml",
    )


def load_intents() -> Dict[str, Any]:
    return json.loads(_core().skills_intents())


def load_pages() -> Dict[str, Any]:
    return json.loads(_core().skills_pages())


def default_parameters(skill: Skill) -> Dict[str, Any]:
    out: Dict[str, Any] = {}
    for p in skill.parameters:
        name = p.get("name")
        if name is not None and "default" in p:
            out[str(name)] = p["default"]
    return out


def expand_skill(
    skill: Skill,
    overrides: Optional[Mapping[str, Any]] = None,
) -> List[SkillStep]:
    """Expand steps via Rust ``skills_plan`` (SSOT)."""
    params = default_parameters(skill)
    if overrides:
        params.update(dict(overrides))
    plan = json.loads(_core().skills_plan(skill.id, json.dumps(params)))
    return [_step_from_raw(raw) for raw in plan.get("steps") or []]


def build_context(
    skill: Skill,
    overrides: Optional[Mapping[str, Any]] = None,
) -> Dict[str, str]:
    plan = json.loads(_core().skills_plan(skill.id, json.dumps(dict(overrides or {}))))
    params = plan.get("parameters") or {}
    return {str(k): str(v) for k, v in params.items()}


def match_skills(query: str, limit: int = 3) -> List[str]:
    return json.loads(_core().skills_match(query, limit))


def validate_skill(skill: Skill) -> List[str]:
    warnings: List[str] = []
    if not skill.steps:
        warnings.append(f"{skill.id}: no steps defined")
    seen_ids: set[str] = set()
    for step in skill.steps:
        if step.id in seen_ids:
            warnings.append(f"{skill.id}: duplicate step id {step.id}")
        seen_ids.add(step.id)
        if step.type == "sql" and not step.sql:
            warnings.append(f"{skill.id}.{step.id}: sql step missing sql")
    skill_md = skill.path.parent / "SKILL.md"
    if not skill_md.is_file():
        warnings.append(f"{skill.id}: missing SKILL.md")
    return warnings


def validate_all() -> List[str]:
    catalog = load_catalog()
    all_warnings: List[str] = []
    for entry in catalog.skills:
        skill = load_skill(entry.id)
        all_warnings.extend(validate_skill(skill))
    return all_warnings

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
    triggers: Dict[str, Any]
    docs: str
    parameters: List[Dict[str, Any]]
    requires: Dict[str, Any]
    steps: List[SkillStep]
    interpretation: Dict[str, Any]
    summary_template: str
    next_steps: List[str]
    variables: Dict[str, str]
    path: Path
    metadata: Dict[str, Any] = field(default_factory=dict)


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


def load_catalog(root: Optional[Path] = None) -> SkillCatalog:
    if root is not None:
        raise NotImplementedError(
            "load_catalog(root=...) is deprecated; use bundled discovery via Rust"
        )
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


def load_skill(skill_id: str, root: Optional[Path] = None) -> Skill:
    if root is not None:
        raise NotImplementedError(
            "load_skill(..., root=...) is deprecated; use bundled discovery via Rust"
        )
    raw = json.loads(_core().skills_load(skill_id))
    if "error" in raw:
        raise KeyError(raw["error"])
    steps = [
        SkillStep(
            id=str(s.get("id", "")),
            title=str(s.get("title", "")),
            type=str(s.get("type", "sql")),
            sql=s.get("sql"),
            method=s.get("method"),
            path=s.get("path"),
            action=s.get("action"),
            view=s.get("view"),
            on_empty=str(s.get("on_empty", "skip")),
            empty_message=s.get("empty_message"),
            when=s.get("when"),
            platform=s.get("platform"),
            raw=dict(s),
        )
        for s in raw.get("steps") or []
    ]
    keywords = raw.get("keywords") or {}
    triggers = {"keywords": keywords}
    return Skill(
        id=str(raw["id"]),
        title=str(raw.get("title", raw["id"])),
        category=str(raw.get("category", "general")),
        tags=list(raw.get("tags") or []),
        triggers=triggers,
        docs=str(raw.get("docs") or "").strip(),
        parameters=list(raw.get("parameters") or []),
        requires={},
        steps=steps,
        interpretation=dict(raw.get("interpretation") or {}),
        summary_template=str(raw.get("summary_template") or "").strip(),
        next_steps=list(raw.get("next_steps") or []),
        variables={},
        path=skills_root() / skill_id / "steps.yaml",
        metadata={"triggers": triggers},
    )


def load_intents(root: Optional[Path] = None) -> Dict[str, Any]:
    if root is not None:
        raise NotImplementedError("load_intents(root=...) is deprecated")
    return json.loads(_core().skills_intents())


def load_pages(root: Optional[Path] = None) -> Dict[str, Any]:
    if root is not None:
        raise NotImplementedError("load_pages(root=...) is deprecated")
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
    out: List[SkillStep] = []
    for raw in plan.get("steps") or []:
        out.append(
            SkillStep(
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
        )
    return out


def build_context(
    skill: Skill,
    overrides: Optional[Mapping[str, Any]] = None,
) -> Dict[str, str]:
    plan = json.loads(_core().skills_plan(skill.id, json.dumps(dict(overrides or {}))))
    params = plan.get("parameters") or {}
    return {str(k): str(v) for k, v in params.items()}


def match_skills(
    query: str,
    root: Optional[Path] = None,
    limit: int = 3,
) -> List[str]:
    if root is not None:
        raise NotImplementedError("match_skills(root=...) is deprecated")
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


def validate_all(root: Optional[Path] = None) -> List[str]:
    catalog = load_catalog(root)
    all_warnings: List[str] = []
    for entry in catalog.skills:
        skill = load_skill(entry.id, root)
        all_warnings.extend(validate_skill(skill))
    return all_warnings

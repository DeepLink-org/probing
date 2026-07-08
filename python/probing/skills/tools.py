"""Agent-callable skill discovery tools (list / plan without prior install).

Skill **execution** and **semantics** live in Rust (``probing-skills``): CLI, Web
WASM, MCP, and these helpers call the shared runtime via ``probing._core``.
"""

from __future__ import annotations

import json
from dataclasses import asdict, dataclass
from typing import Any, Dict, List, Mapping, Optional

import probing._core as core


@dataclass
class SkillSummary:
    id: str
    category: str
    description: str
    priority: int
    title: str = ""

    def to_dict(self) -> Dict[str, Any]:
        return asdict(self)


@dataclass
class SkillRunPlan:
    skill_id: str
    title: str
    docs: str
    parameters: Dict[str, Any]
    steps: List[Dict[str, Any]]
    summary_template: str
    next_steps: List[str]
    cli_command: str

    def to_dict(self) -> Dict[str, Any]:
        return asdict(self)


def list_skills(*, query: Optional[str] = None, limit: int = 20) -> List[SkillSummary]:
    raw = json.loads(core.skills_list(query, limit))
    return [
        SkillSummary(
            id=str(item["id"]),
            category=str(item.get("category", "")),
            description=str(item.get("description", "")),
            priority=int(item.get("priority", 0)),
            title=str(item.get("title", item["id"])),
        )
        for item in raw
    ]


def list_skills_json(*, query: Optional[str] = None, limit: int = 20) -> str:
    return core.skills_list(query, limit)


def plan_skill_run(
    skill_id: str,
    params: Optional[Mapping[str, Any]] = None,
    *,
    target: Optional[str] = None,
) -> SkillRunPlan:
    _ = target
    payload = params or {}
    plan = json.loads(core.skills_plan(skill_id, json.dumps(dict(payload))))
    skill = json.loads(core.skills_load(skill_id))
    cli = _format_cli_command(skill_id, plan.get("parameters") or payload, target)
    return SkillRunPlan(
        skill_id=str(plan.get("skill_id", skill_id)),
        title=str(plan.get("title", skill.get("title", skill_id))),
        docs=str(skill.get("docs", "")),
        parameters=dict(plan.get("parameters") or {}),
        steps=list(plan.get("steps") or []),
        summary_template=str(skill.get("summary_template", "")),
        next_steps=list(skill.get("next_steps") or []),
        cli_command=cli,
    )


def _format_cli_command(
    skill_id: str,
    params: Mapping[str, Any],
    target: Optional[str],
) -> str:
    parts = ["probing"]
    if target:
        parts.extend(["-t", str(target)])
    parts.extend(["skill", "run", skill_id])
    for key, value in sorted(params.items()):
        parts.extend(["--set", f"{key}={value}"])
    return " ".join(parts)

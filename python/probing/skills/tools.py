"""Agent-callable skill tools (list / run without prior install)."""

from __future__ import annotations

import json
import shutil
import subprocess
from dataclasses import asdict, dataclass
from typing import Any, Dict, List, Mapping, Optional, Sequence

from probing.skills.loader import (
    Skill,
    SkillStep,
    default_parameters,
    expand_skill,
    load_catalog,
    load_skill,
    match_skills,
)


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


def _step_to_dict(step: SkillStep) -> Dict[str, Any]:
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
        "when": step.when,
    }


def list_skills(*, query: Optional[str] = None, limit: int = 20) -> List[SkillSummary]:
    """List bundled and installed skills (no ``probing skill install`` required)."""
    catalog = load_catalog()
    summaries: List[SkillSummary] = []
    for entry in catalog.skills:
        title = entry.id
        try:
            skill = load_skill(entry.id)
            title = skill.title
        except KeyError:
            pass
        summaries.append(
            SkillSummary(
                id=entry.id,
                category=entry.category,
                description=entry.description,
                priority=entry.priority,
                title=title,
            )
        )
    if query:
        ranked = match_skills(query, limit=limit)
        by_id = {s.id: s for s in summaries}
        ordered = [by_id[sid] for sid in ranked if sid in by_id]
        if ordered:
            return ordered[:limit]
    return summaries[:limit]


def list_skills_json(*, query: Optional[str] = None, limit: int = 20) -> str:
    return json.dumps(
        [s.to_dict() for s in list_skills(query=query, limit=limit)], indent=2
    )


def plan_skill_run(
    skill_id: str,
    params: Optional[Mapping[str, Any]] = None,
    *,
    target: Optional[str] = None,
) -> SkillRunPlan:
    """Expand a skill into executable steps (uses bundled skills by default)."""
    skill = load_skill(skill_id)
    merged = default_parameters(skill)
    if params:
        merged.update(dict(params))
    steps = expand_skill(skill, merged)
    cli = _format_cli_command(skill_id, merged, target)
    return SkillRunPlan(
        skill_id=skill.id,
        title=skill.title,
        docs=skill.docs,
        parameters=merged,
        steps=[_step_to_dict(s) for s in steps],
        summary_template=skill.summary_template,
        next_steps=list(skill.next_steps),
        cli_command=cli,
    )


def run_skill(
    skill_id: str,
    *,
    target: Optional[str] = None,
    params: Optional[Mapping[str, Any]] = None,
    global_fanout: Optional[bool] = None,
    execute: bool = True,
) -> Dict[str, Any]:
    """Run or plan a skill.

    When *target* is set and *execute* is true, invokes the ``probing`` CLI.
    Otherwise returns an expanded step plan for the agent to follow.
    """
    merged: Dict[str, Any] = dict(params or {})
    if global_fanout is not None:
        merged["use_global"] = global_fanout

    if target and execute:
        probing = shutil.which("probing")
        if probing is None:
            plan = plan_skill_run(skill_id, merged, target=target)
            return {
                "status": "plan_only",
                "reason": "probing CLI not found on PATH",
                "plan": plan.to_dict(),
            }
        cmd = [probing]
        if target:
            cmd.extend(["-t", target])
        cmd.extend(["skill", "run", skill_id])
        for key, value in merged.items():
            cmd.extend(["--set", f"{key}={value}"])
        if global_fanout is True:
            cmd.append("--global")
        elif global_fanout is False:
            cmd.append("--local")
        proc = subprocess.run(cmd, capture_output=True, text=True, check=False)
        return {
            "status": "ok" if proc.returncode == 0 else "error",
            "returncode": proc.returncode,
            "stdout": proc.stdout,
            "stderr": proc.stderr,
            "command": " ".join(cmd),
        }

    plan = plan_skill_run(skill_id, merged, target=target)
    return {"status": "plan", "plan": plan.to_dict()}


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


def run_skill_json(
    skill_id: str,
    *,
    target: Optional[str] = None,
    params: Optional[Mapping[str, Any]] = None,
    global_fanout: Optional[bool] = None,
    execute: bool = True,
) -> str:
    return json.dumps(
        run_skill(
            skill_id,
            target=target,
            params=params,
            global_fanout=global_fanout,
            execute=execute,
        ),
        indent=2,
    )

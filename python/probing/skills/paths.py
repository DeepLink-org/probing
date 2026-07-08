"""Agent skill install paths and re-exports from ``probing.extensions``."""

from __future__ import annotations

import shutil
from dataclasses import dataclass
from pathlib import Path
from typing import Dict, Iterable, List, Optional, Sequence

from probing.extensions import (
    SkillRoot,
    bundled_skills_dir,
    default_install_source,
    find_repo_root,
    repo_skills_dir,
    resolve_skill_dir,
    skill_root_bundled,
    skill_roots,
)

PROBING_PROJECT_SKILLS = ".probing/skills"
PROBING_USER_SKILLS = ".probing/skills"
REPO_SKILLS_DIRNAME = "skills"

AGENT_PROJECT_SKILLS: Dict[str, str] = {
    "cursor": ".cursor/skills",
    "claude": ".claude/skills",
    "codex": ".agents/skills",
}
AGENT_USER_SKILLS: Dict[str, str] = {
    "cursor": ".cursor/skills",
    "claude": ".claude/skills",
    "codex": ".agents/skills",
}
AGENT_PROJECT_MARKERS: Dict[str, tuple[str, ...]] = {
    "cursor": (".cursor",),
    "claude": (".claude",),
    "codex": (".agents", ".codex"),
}
AGENT_BINARIES: Dict[str, tuple[str, ...]] = {
    "cursor": ("cursor",),
    "claude": ("claude",),
    "codex": ("codex",),
}
ALL_AGENTS = ("cursor", "claude", "codex")


@dataclass(frozen=True)
class AgentInstallTarget:
    agent: str
    scope: str  # "project" | "user"
    skills_dir: Path
    reason: str


def _binary_available(name: str) -> bool:
    return shutil.which(name) is not None


def detect_agent_presence(start: Optional[Path] = None) -> Dict[str, bool]:
    start = (start or Path.cwd()).resolve()
    home = Path.home()
    presence: Dict[str, bool] = {}

    for agent in ALL_AGENTS:
        markers = AGENT_PROJECT_MARKERS[agent]
        project_hit = any(
            (directory / marker).is_dir()
            for directory in (start, *start.parents)
            for marker in markers
        )
        user_hit = any((home / marker).is_dir() for marker in markers)
        user_skills = home / AGENT_USER_SKILLS[agent]
        binary_hit = any(_binary_available(name) for name in AGENT_BINARIES[agent])
        presence[agent] = project_hit or user_hit or user_skills.is_dir() or binary_hit

    return presence


def _project_root_for_agent(start: Path, agent: str) -> Optional[Path]:
    markers = AGENT_PROJECT_MARKERS[agent]
    for directory in (start, *start.parents):
        if any((directory / marker).is_dir() for marker in markers):
            return directory
        if directory.parent == directory:
            break
    return find_repo_root(start)


def detect_agent_install_targets(
    start: Optional[Path] = None,
    *,
    user: bool = False,
    agents: Optional[Sequence[str]] = None,
    force: bool = False,
) -> List[AgentInstallTarget]:
    start = (start or Path.cwd()).resolve()
    requested = [a for a in (agents or ALL_AGENTS) if a in ALL_AGENTS]
    presence = detect_agent_presence(start)
    targets: List[AgentInstallTarget] = []

    for agent in requested:
        if not force and not presence.get(agent, False):
            continue

        if user:
            skills_dir = Path.home() / AGENT_USER_SKILLS[agent]
            reason = f"{agent} user config detected"
            targets.append(AgentInstallTarget(agent, "user", skills_dir, reason))
            continue

        root = _project_root_for_agent(start, agent) or find_repo_root(start) or start
        rel = AGENT_PROJECT_SKILLS[agent]
        skills_dir = root / rel
        if presence.get(agent, False):
            reason = f"{agent} project marker under {root}"
        else:
            reason = f"{agent} requested; installing under {root}"
        targets.append(AgentInstallTarget(agent, "project", skills_dir, reason))

    deduped: List[AgentInstallTarget] = []
    seen: set[str] = set()
    for target in targets:
        key = str(target.skills_dir.resolve())
        if key in seen:
            continue
        seen.add(key)
        deduped.append(target)
    return deduped


def iter_skill_ids_in_root(root: Path) -> Iterable[str]:
    catalog = root / "catalog.yaml"
    if catalog.is_file():
        try:
            import yaml
        except ImportError:
            yaml = None  # type: ignore
        if yaml is not None:
            data = yaml.safe_load(catalog.read_text(encoding="utf-8")) or {}
            for entry in data.get("skills") or []:
                sid = entry.get("id")
                if sid:
                    yield str(sid)
            return
    for child in sorted(root.iterdir()):
        if child.is_dir() and (child / "SKILL.md").is_file():
            yield child.name


__all__ = [
    "AGENT_BINARIES",
    "AGENT_PROJECT_MARKERS",
    "AGENT_PROJECT_SKILLS",
    "AGENT_USER_SKILLS",
    "ALL_AGENTS",
    "AgentInstallTarget",
    "PROBING_PROJECT_SKILLS",
    "PROBING_USER_SKILLS",
    "REPO_SKILLS_DIRNAME",
    "SkillRoot",
    "bundled_skills_dir",
    "default_install_source",
    "detect_agent_install_targets",
    "detect_agent_presence",
    "find_repo_root",
    "iter_skill_ids_in_root",
    "repo_skills_dir",
    "resolve_skill_dir",
    "skill_root_bundled",
    "skill_roots",
]

"""Agent skill directory discovery (Cursor, Claude Code, Codex)."""

from __future__ import annotations

import os
import shutil
from dataclasses import dataclass
from pathlib import Path
from typing import Dict, Iterable, List, Optional, Sequence

PROBING_PROJECT_SKILLS = ".probing/skills"
PROBING_USER_SKILLS = ".probing/skills"
REPO_SKILLS_DIRNAME = "skills"

# Where each agent runtime loads project/user skills (Agent Skills open standard).
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
class SkillRoot:
    """A directory tree that contains skill folders and optionally catalog.yaml."""

    path: Path
    label: str


@dataclass(frozen=True)
class AgentInstallTarget:
    """A directory where probing skills should be installed for an agent runtime."""

    agent: str
    scope: str  # "project" | "user"
    skills_dir: Path
    reason: str


def find_repo_root(start: Optional[Path] = None) -> Optional[Path]:
    """Find probing repository root (contains ``skills/catalog.yaml`` + ``pyproject.toml``)."""
    start = (start or Path.cwd()).resolve()
    for directory in (start, *start.parents):
        if (directory / REPO_SKILLS_DIRNAME / "catalog.yaml").is_file() and (
            directory / "pyproject.toml"
        ).is_file():
            return directory
        if directory.parent == directory:
            break
    # Source checkout: python/probing/skills/paths.py → repo root is parents[3]
    pkg_root = Path(__file__).resolve().parents[3]
    if (pkg_root / REPO_SKILLS_DIRNAME / "catalog.yaml").is_file():
        return pkg_root
    return None


def repo_skills_dir(start: Optional[Path] = None) -> Optional[Path]:
    """Top-level ``skills/`` in a probing checkout (authoring + agent install source)."""
    root = find_repo_root(start)
    if root is None:
        return None
    candidate = root / REPO_SKILLS_DIRNAME
    return candidate if (candidate / "catalog.yaml").is_file() else None


def bundled_skills_dir() -> Optional[Path]:
    """Skills copied into the wheel at ``python/probing/_skills/`` (release builds)."""
    root = _package_dir() / "_skills"
    if root.is_dir() and (root / "catalog.yaml").is_file():
        return root
    return _resource_dir("_skills", "catalog.yaml")


def _package_dir() -> Path:
    return Path(__file__).resolve().parent.parent


def _resource_dir(name: str, marker: str) -> Optional[Path]:
    try:
        from importlib.resources import as_file, files

        bundle = files("probing") / name
        if not (bundle / marker).is_file():
            return None
        with as_file(bundle) as path:
            return Path(path)
    except (TypeError, ModuleNotFoundError, FileNotFoundError, OSError):
        return None


def user_skills_dir() -> Path:
    return Path.home() / PROBING_USER_SKILLS


def project_skills_dir(start: Optional[Path] = None) -> Optional[Path]:
    start = start or Path.cwd()
    for directory in (start, *start.parents):
        candidate = directory / PROBING_PROJECT_SKILLS
        if candidate.is_dir():
            return candidate
        if directory.parent == directory:
            break
    return None


def env_skills_dir() -> Optional[Path]:
    raw = os.environ.get("PROBING_SKILLS_DIR")
    if not raw:
        return None
    path = Path(raw).expanduser().resolve()
    return path if path.is_dir() else None


def skill_roots(start: Optional[Path] = None) -> List[SkillRoot]:
    """Return skill roots from lowest to highest priority (later overrides earlier)."""
    roots: List[SkillRoot] = []

    bundled = bundled_skills_dir()
    if bundled is not None:
        roots.append(SkillRoot(bundled, "bundled"))

    repo = repo_skills_dir(start)
    if repo is not None and not any(r.path.resolve() == repo.resolve() for r in roots):
        roots.append(SkillRoot(repo, "repo"))

    user = user_skills_dir()
    if user.is_dir():
        roots.append(SkillRoot(user, "user"))

    project = project_skills_dir(start)
    if project is not None:
        roots.append(SkillRoot(project, "project"))

    extra = env_skills_dir()
    if extra is not None:
        roots.append(SkillRoot(extra, "env"))

    return roots


def default_install_source(start: Optional[Path] = None) -> Path:
    """Directory copied by ``probing skill install`` / ``skills/install.sh``."""
    repo = repo_skills_dir(start)
    if repo is not None:
        return repo
    bundled = bundled_skills_dir()
    if bundled is not None:
        return bundled
    raise FileNotFoundError(
        "No skills/ directory found. Run from the probing repo or install probing from a wheel."
    )


def _binary_available(name: str) -> bool:
    return shutil.which(name) is not None


def detect_agent_presence(start: Optional[Path] = None) -> Dict[str, bool]:
    """Return which agent runtimes appear to be in use for *start* / home."""
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
    """Resolve install destinations for Cursor / Claude Code / Codex."""
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


def resolve_skill_dir(skill_id: str, roots: Sequence[SkillRoot]) -> Optional[Path]:
    for root in reversed(roots):
        catalog_path = None
        catalog_file = root.path / "catalog.yaml"
        if catalog_file.is_file():
            try:
                import yaml
            except ImportError:
                yaml = None  # type: ignore
            if yaml is not None:
                data = yaml.safe_load(catalog_file.read_text(encoding="utf-8")) or {}
                for entry in data.get("skills") or []:
                    if str(entry.get("id")) == skill_id:
                        rel = entry.get("path") or entry.get("file")
                        if rel:
                            catalog_path = root.path / str(rel)
                            break
        if catalog_path is not None and catalog_path.is_file():
            return catalog_path.parent
        direct = root.path / skill_id
        if (direct / "steps.yaml").is_file() or (direct / "SKILL.md").is_file():
            return direct
    return None

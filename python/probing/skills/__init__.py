"""Probing diagnostic skills — discovery via Rust (``probing-skills``), install via Python."""

from probing.skills.loader import (
    Skill,
    SkillCatalog,
    SkillStep,
    expand_skill,
    load_catalog,
    load_intents,
    load_pages,
    load_skill,
    match_skills,
    skills_root,
    validate_all,
)
from probing.skills.install import install_skills
from probing.skills.paths import (
    AgentInstallTarget,
    ALL_AGENTS,
    bundled_skills_dir,
    detect_agent_install_targets,
    detect_agent_presence,
    find_repo_root,
    repo_skills_dir,
    skill_roots,
)
from probing.skills.tools import (
    list_skills,
    list_skills_json,
    plan_skill_run,
)

__all__ = [
    "Skill",
    "SkillCatalog",
    "SkillStep",
    "AgentInstallTarget",
    "ALL_AGENTS",
    "bundled_skills_dir",
    "detect_agent_install_targets",
    "detect_agent_presence",
    "find_repo_root",
    "repo_skills_dir",
    "expand_skill",
    "install_skills",
    "list_skills",
    "list_skills_json",
    "load_catalog",
    "load_intents",
    "load_pages",
    "load_skill",
    "match_skills",
    "plan_skill_run",
    "skill_roots",
    "skills_root",
    "validate_all",
]

"""Skill root discovery — entry points plus filesystem overlays."""

from __future__ import annotations

import json
import os
from pathlib import Path
from typing import List, Optional, Sequence

from probing.extensions.entrypoints import load_skill_roots_from_entry_points
from probing.extensions.types import SkillRoot

REPO_SKILLS_DIRNAME = "skills"
PROBING_PROJECT_SKILLS = ".probing/skills"
PROBING_USER_SKILLS = ".probing/skills"


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


def bundled_skills_dir() -> Optional[Path]:
    """SSOT: ``python/probing/bundled_skills`` (repo ``skills/`` symlink).

    When running inside a probing git checkout, prefer the source tree over a
    wheel copy under site-packages so dev/CI edits to ``bundled_skills/`` match
    what discovery returns.
    """
    repo = find_repo_root()
    if repo is not None:
        checkout = repo / "python" / "probing" / "bundled_skills"
        if (checkout / "catalog.yaml").is_file():
            return checkout
    root = _package_dir() / "bundled_skills"
    if (root / "catalog.yaml").is_file():
        return root
    return _resource_dir("bundled_skills", "catalog.yaml")


def skill_root_bundled() -> Path:
    """``probing.skills`` entry point for the core bundled catalog."""
    root = bundled_skills_dir()
    if root is None:
        raise RuntimeError(
            "bundled skills not found; install probing (pip install -e . or wheel)"
        )
    return root


def find_repo_root(start: Optional[Path] = None) -> Optional[Path]:
    start = (start or Path.cwd()).resolve()
    for directory in (start, *start.parents):
        if (directory / REPO_SKILLS_DIRNAME / "catalog.yaml").is_file() and (
            directory / "pyproject.toml"
        ).is_file():
            return directory
        if directory.parent == directory:
            break
    pkg_root = Path(__file__).resolve().parents[3]
    if (pkg_root / REPO_SKILLS_DIRNAME / "catalog.yaml").is_file():
        return pkg_root
    return None


def repo_skills_dir(start: Optional[Path] = None) -> Optional[Path]:
    root = find_repo_root(start)
    if root is None:
        return None
    candidate = root / REPO_SKILLS_DIRNAME
    return candidate if (candidate / "catalog.yaml").is_file() else None


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
    """Return skill roots from lowest to highest priority (later overrides earlier).

    Order: bundled (filesystem fallback) → repo checkout → entry points →
    user → project → ``PROBING_SKILLS_DIR``.
    """
    roots: List[SkillRoot] = []
    seen: set[str] = set()

    def _add(path: Path, label: str) -> None:
        try:
            key = str(path.resolve())
        except OSError:
            key = str(path)
        if key in seen:
            return
        seen.add(key)
        roots.append(SkillRoot(path, label))

    bundled = bundled_skills_dir()
    if bundled is not None:
        _add(bundled, "bundled")

    repo = repo_skills_dir(start)
    if repo is not None:
        _add(repo, "repo")

    for path, label in load_skill_roots_from_entry_points():
        _add(path, label)

    user = user_skills_dir()
    if user.is_dir():
        _add(user, "user")

    project = project_skills_dir(start)
    if project is not None:
        _add(project, "project")

    extra = env_skills_dir()
    if extra is not None:
        _add(extra, "env")

    return roots


def default_install_source(start: Optional[Path] = None) -> Path:
    repo = repo_skills_dir(start)
    if repo is not None:
        return repo
    bundled = bundled_skills_dir()
    if bundled is not None:
        return bundled
    raise FileNotFoundError(
        "No skills/ directory found. Run from the probing repo or install probing from a wheel."
    )


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


def skill_roots_json() -> str:
    payload = [{"path": str(root.path), "label": root.label} for root in skill_roots()]
    return json.dumps(payload)

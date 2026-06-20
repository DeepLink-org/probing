"""Tests for skill paths, install, and tools."""

from __future__ import annotations

import json
from pathlib import Path

import pytest

pytest.importorskip("yaml")

from probing.skills.install import install_skills
from probing.skills.loader import load_catalog
from probing.skills.paths import (
    bundled_skills_dir,
    detect_agent_install_targets,
    repo_skills_dir,
    skill_roots,
)
from probing.skills.tools import list_skills, plan_skill_run, run_skill

from tests.conftest import is_wheel_install


def test_bundled_skills_dir_exists():
    bundled = bundled_skills_dir()
    if is_wheel_install():
        assert bundled is not None, "installed wheel is missing probing/bundled_skills"
        assert (bundled / "catalog.yaml").is_file()
        return
    assert bundled is not None or repo_skills_dir() is not None
    if bundled is not None:
        assert (bundled / "catalog.yaml").is_file()


def test_skill_roots_include_bundled():
    labels = [r.label for r in skill_roots()]
    if is_wheel_install():
        assert "bundled" in labels
        return
    assert "bundled" in labels or "repo" in labels


def test_merged_catalog_has_eight_skills():
    catalog = load_catalog()
    assert len(catalog.skills) == 8


def test_list_skills_tool():
    skills = list_skills(limit=10)
    assert len(skills) >= 8
    assert any(s.id == "health_overview" for s in skills)


def test_plan_skill_run():
    plan = plan_skill_run("health_overview")
    assert plan.skill_id == "health_overview"
    assert plan.steps
    assert "probing" in plan.cli_command


def test_run_skill_plan_only():
    result = run_skill("health_overview", execute=False)
    assert result["status"] == "plan"
    assert "plan" in result


def test_detect_cursor_project_target(tmp_path: Path):
    (tmp_path / ".cursor").mkdir()
    targets = detect_agent_install_targets(tmp_path)
    assert any(
        t.agent == "cursor" and t.skills_dir == tmp_path / ".cursor" / "skills"
        for t in targets
    )


def test_detect_codex_uses_agents_skills(tmp_path: Path):
    (tmp_path / ".codex").mkdir()
    targets = detect_agent_install_targets(tmp_path)
    assert any(
        t.agent == "codex" and t.skills_dir == tmp_path / ".agents" / "skills"
        for t in targets
    )


def test_install_to_cursor_project(tmp_path: Path):
    (tmp_path / ".cursor").mkdir()
    manifest = install_skills(cwd=tmp_path, agents=["cursor"])
    assert manifest.targets
    skill_dir = tmp_path / ".cursor" / "skills" / "health_overview"
    assert (skill_dir / "SKILL.md").is_file()


def test_install_user_scope(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    home = tmp_path / "home"
    home.mkdir()
    monkeypatch.setenv("HOME", str(home))
    (home / ".claude").mkdir()
    manifest = install_skills(cwd=tmp_path, user=True, agents=["claude"])
    assert any(
        t["agent"] == "claude" and t["scope"] == "user" for t in manifest.targets
    )
    assert (home / ".claude" / "skills" / "health_overview" / "SKILL.md").is_file()


def test_list_skills_json():
    from probing.skills.tools import list_skills_json

    payload = json.loads(list_skills_json(limit=3))
    assert isinstance(payload, list)
    assert payload

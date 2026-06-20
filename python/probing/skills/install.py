"""Install probing skills into Cursor, Claude Code, and Codex skill directories."""

from __future__ import annotations

import argparse
import json
import shutil
import sys
from dataclasses import asdict, dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import List, Optional, Sequence

from probing import VERSION as probing_version

from probing.skills.paths import (
    ALL_AGENTS,
    AgentInstallTarget,
    default_install_source,
    detect_agent_install_targets,
    detect_agent_presence,
    iter_skill_ids_in_root,
)

MANIFEST_NAME = ".probing-skills-install.json"


@dataclass
class InstallManifest:
    version: str
    source: str
    skills: List[str]
    targets: List[dict]
    installed_at: str

    @classmethod
    def load(cls, path: Path) -> Optional["InstallManifest"]:
        if not path.is_file():
            return None
        data = json.loads(path.read_text(encoding="utf-8"))
        return cls(**data)

    def save(self, path: Path) -> None:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(asdict(self), indent=2) + "\n", encoding="utf-8")


def _copy_skill_tree(src: Path, dest: Path) -> None:
    if dest.exists():
        if dest.is_symlink() or dest.is_file():
            dest.unlink()
        else:
            shutil.rmtree(dest)
    shutil.copytree(src, dest, symlinks=False)


def _skill_source_dirs(
    source: Path, skill_ids: Optional[Sequence[str]] = None
) -> List[tuple[str, Path]]:
    ids = list(skill_ids) if skill_ids else list(iter_skill_ids_in_root(source))
    out: List[tuple[str, Path]] = []
    for sid in ids:
        direct = source / sid
        if (direct / "steps.yaml").is_file() or (direct / "SKILL.md").is_file():
            out.append((sid, direct))
            continue
        try:
            import yaml
        except ImportError:
            continue
        catalog = source / "catalog.yaml"
        if not catalog.is_file():
            continue
        data = yaml.safe_load(catalog.read_text(encoding="utf-8")) or {}
        for entry in data.get("skills") or []:
            if str(entry.get("id")) != sid:
                continue
            rel = entry.get("path") or entry.get("file")
            if rel:
                path = source / str(rel)
                out.append((sid, path.parent))
            break
    return out


def _manifest_path(cwd: Path, user: bool) -> Path:
    if user:
        return Path.home() / ".probing" / MANIFEST_NAME
    return cwd / ".probing" / MANIFEST_NAME


def _remove_installed_skills(
    target: AgentInstallTarget, skill_ids: Sequence[str]
) -> None:
    if not target.skills_dir.is_dir():
        return
    for sid in skill_ids:
        dest = target.skills_dir / sid
        if dest.is_symlink() or dest.is_file():
            dest.unlink()
        elif dest.is_dir():
            shutil.rmtree(dest)


def install_skills(
    *,
    user: bool = False,
    update: bool = False,
    source: Optional[Path] = None,
    cwd: Optional[Path] = None,
    agents: Optional[Sequence[str]] = None,
    force: bool = False,
    skill_ids: Optional[Sequence[str]] = None,
) -> InstallManifest:
    """Install bundled skills into detected Cursor / Claude / Codex skill directories."""
    cwd = (cwd or Path.cwd()).resolve()
    source = (source or default_install_source()).resolve()
    if not source.is_dir():
        raise FileNotFoundError(f"Skill source not found: {source}")

    targets = detect_agent_install_targets(
        cwd,
        user=user,
        agents=agents,
        force=force,
    )
    if not targets:
        presence = detect_agent_presence(cwd)
        detected = [name for name, ok in presence.items() if ok]
        hint = ", ".join(detected) if detected else "none"
        raise FileNotFoundError(
            "No Cursor, Claude Code, or Codex skill directories to install into. "
            f"Detected: {hint}. Use --force to create standard paths anyway, "
            "or --agent cursor,claude,codex to narrow targets."
        )

    manifest_path = _manifest_path(cwd, user)
    if update and manifest_path.is_file():
        old = InstallManifest.load(manifest_path)
        if old is not None:
            for rec in old.targets:
                target = AgentInstallTarget(
                    rec["agent"],
                    rec["scope"],
                    Path(rec["path"]),
                    rec.get("reason", ""),
                )
                _remove_installed_skills(target, old.skills)

    pairs = _skill_source_dirs(source, skill_ids)
    installed_ids = [sid for sid, _ in pairs]

    target_records: List[dict] = []
    for target in targets:
        target.skills_dir.mkdir(parents=True, exist_ok=True)
        for sid, src_dir in pairs:
            _copy_skill_tree(src_dir, target.skills_dir / sid)
        target_records.append(
            {
                "agent": target.agent,
                "scope": target.scope,
                "path": str(target.skills_dir.resolve()),
                "reason": target.reason,
            }
        )

    manifest = InstallManifest(
        version=probing_version,
        source=str(source),
        skills=installed_ids,
        targets=target_records,
        installed_at=datetime.now(timezone.utc).isoformat(),
    )
    manifest.save(manifest_path)
    return manifest


def _parse_agents(raw: Optional[str]) -> Optional[List[str]]:
    if not raw:
        return None
    names = [part.strip().lower() for part in raw.split(",") if part.strip()]
    unknown = [name for name in names if name not in ALL_AGENTS]
    if unknown:
        raise ValueError(
            f"Unknown agent(s): {', '.join(unknown)} (expected cursor, claude, codex)"
        )
    return names


def main(argv: Optional[Sequence[str]] = None) -> int:
    parser = argparse.ArgumentParser(
        description="Install probing diagnostic skills for Cursor, Claude Code, and Codex"
    )
    parser.add_argument(
        "action",
        nargs="?",
        default="install",
        choices=("install", "update"),
        help="install or update (re-sync from bundled source)",
    )
    parser.add_argument(
        "--user",
        action="store_true",
        help="Install to user-level dirs (~/.cursor/skills, ~/.claude/skills, ~/.agents/skills)",
    )
    parser.add_argument(
        "--from", dest="from_path", metavar="PATH", help="Skill source directory"
    )
    parser.add_argument(
        "--update", action="store_true", help="Re-sync existing install"
    )
    parser.add_argument(
        "--agent",
        metavar="NAMES",
        help="Comma-separated agents: cursor, claude, codex",
    )
    parser.add_argument(
        "--force",
        action="store_true",
        help="Install even when agent markers were not detected (creates standard paths)",
    )
    args = parser.parse_args(list(argv) if argv is not None else None)

    source = Path(args.from_path).expanduser().resolve() if args.from_path else None
    try:
        agent_filter = _parse_agents(args.agent)
        manifest = install_skills(
            user=args.user,
            update=args.action == "update" or args.update,
            source=source,
            agents=agent_filter,
            force=args.force,
        )
    except (FileNotFoundError, ValueError) as exc:
        print(str(exc), file=sys.stderr)
        return 1

    presence = detect_agent_presence()
    detected = [name for name, ok in presence.items() if ok]
    print(
        f"Installed {len(manifest.skills)} skills for {len(manifest.targets)} target(s)"
    )
    for rec in manifest.targets:
        print(f"  {rec['agent']} ({rec['scope']}): {rec['path']}")
    if not manifest.targets:
        print(f"Detected agents: {', '.join(detected) or 'none'}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

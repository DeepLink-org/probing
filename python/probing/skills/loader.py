"""Load and expand probing diagnostic skills (SKILL.md + steps.yaml).

Requires PyYAML: ``pip install pyyaml`` (optional; only needed for skill tooling).
"""

from __future__ import annotations

import re
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Dict, Iterable, List, Mapping, Optional, Sequence, Union

_PLACEHOLDER = re.compile(r"\{([a-zA-Z_][a-zA-Z0-9_]*)\}")


def _yaml_load(text: str) -> Any:
    try:
        import yaml
    except ImportError as e:
        raise ImportError(
            "Skill loading requires PyYAML. Install with: pip install pyyaml"
        ) from e
    return yaml.safe_load(text)


def skills_root() -> Path:
    """Return the highest-priority skills directory (env > project > user > repo > bundled)."""
    from probing.skills.paths import skill_roots

    roots = skill_roots()
    if roots:
        return roots[-1].path
    repo = Path(__file__).resolve().parents[3] / "skills"
    return repo


def _load_catalog_file(root: Path) -> SkillCatalog:
    catalog_path = root / "catalog.yaml"
    if not catalog_path.is_file():
        raise FileNotFoundError(catalog_path)
    data = _yaml_load(catalog_path.read_text(encoding="utf-8"))
    entries: List[SkillCatalogEntry] = []
    for p in data.get("skills") or []:
        path = p.get("path") or p.get("file", "")
        entries.append(
            SkillCatalogEntry(
                id=str(p["id"]),
                path=str(path),
                category=str(p.get("category", "")),
                priority=int(p.get("priority", 0)),
                description=str(p.get("description", "")),
            )
        )
    entries.sort(key=lambda e: e.priority)
    return SkillCatalog(skills=entries)


def _semantic_file(name: str, root: Optional[Path] = None) -> Path:
    from probing.skills.paths import skill_roots

    if root is not None:
        return root / "semantic" / name
    for skill_root in reversed(skill_roots()):
        path = skill_root.path / "semantic" / name
        if path.is_file():
            return path
    return skills_root() / "semantic" / name


def load_catalog(root: Optional[Path] = None) -> SkillCatalog:
    if root is not None:
        return _load_catalog_file(root)

    from probing.skills.paths import skill_roots

    merged: Dict[str, SkillCatalogEntry] = {}
    for skill_root in skill_roots():
        try:
            catalog = _load_catalog_file(skill_root.path)
        except FileNotFoundError:
            continue
        for entry in catalog.skills:
            merged[entry.id] = entry
    entries = sorted(merged.values(), key=lambda e: (e.priority, e.id))
    return SkillCatalog(skills=entries)


def load_skill(skill_id: str, root: Optional[Path] = None) -> Skill:
    if root is not None:
        catalog = _load_catalog_file(root)
        entry = next((p for p in catalog.skills if p.id == skill_id), None)
        if entry is None:
            raise KeyError(f"Unknown skill: {skill_id}")
        path = root / entry.path
        data = _yaml_load(path.read_text(encoding="utf-8"))
        return _parse_skill_steps(data, path)

    from probing.skills.paths import resolve_skill_dir, skill_roots

    roots = skill_roots()
    skill_dir = resolve_skill_dir(skill_id, roots)
    if skill_dir is None:
        raise KeyError(f"Unknown skill: {skill_id}")
    steps_path = skill_dir / "steps.yaml"
    if not steps_path.is_file():
        raise FileNotFoundError(
            f"Missing steps.yaml for skill {skill_id}: {steps_path}"
        )
    data = _yaml_load(steps_path.read_text(encoding="utf-8"))
    return _parse_skill_steps(data, steps_path)


def load_semantic_catalog(root: Optional[Path] = None) -> Dict[str, Any]:
    path = _semantic_file("tables.yaml", root)
    return _yaml_load(path.read_text(encoding="utf-8"))


def load_intents(root: Optional[Path] = None) -> Dict[str, Any]:
    path = _semantic_file("intents.yaml", root)
    return _yaml_load(path.read_text(encoding="utf-8"))


def load_pages(root: Optional[Path] = None) -> Dict[str, Any]:
    path = _semantic_file("pages.yaml", root)
    return _yaml_load(path.read_text(encoding="utf-8"))


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


def _parse_skill_steps(data: Mapping[str, Any], path: Path) -> Skill:
    meta = data.get("metadata") or {}
    spec = data.get("spec") or {}
    steps: List[SkillStep] = []
    for raw in spec.get("steps") or []:
        steps.append(
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
    return Skill(
        id=str(meta.get("id", path.parent.name)),
        title=str(meta.get("title", meta.get("id", path.parent.name))),
        category=str(meta.get("category", "general")),
        tags=list(meta.get("tags") or []),
        triggers=dict(meta.get("triggers") or {}),
        docs=str(meta.get("docs") or "").strip(),
        parameters=list(spec.get("parameters") or []),
        requires=dict(spec.get("requires") or {}),
        steps=steps,
        interpretation=dict(spec.get("interpretation") or {}),
        summary_template=str(spec.get("summary_template") or "").strip(),
        next_steps=list(spec.get("next_steps") or []),
        variables=dict(spec.get("variables") or {}),
        path=path,
        metadata=dict(meta),
    )


def default_parameters(skill: Skill) -> Dict[str, Any]:
    out: Dict[str, Any] = {}
    for p in skill.parameters:
        name = p.get("name")
        if name is not None and "default" in p:
            out[str(name)] = p["default"]
    return out


def derived_variables(params: Mapping[str, Any]) -> Dict[str, str]:
    use_global = bool(params.get("use_global", False))
    comm = "global.python.comm_collective" if use_global else "python.comm_collective"
    nccl_proxy = "global.nccl.proxy_ops" if use_global else "nccl.proxy_ops"
    net_qp = "global.nccl.net_qp" if use_global else "nccl.net_qp"
    return {
        "comm_table": comm,
        "table_comm": comm,
        "nccl_proxy_table": nccl_proxy,
        "net_qp_table": net_qp,
        "global_prefix": "global." if use_global else "",
    }


def _expand_string(template: str, ctx: Mapping[str, Any]) -> str:
    def repl(match: re.Match[str]) -> str:
        key = match.group(1)
        if key not in ctx:
            raise KeyError(f"Missing skill parameter or variable: {key}")
        return str(ctx[key])

    return _PLACEHOLDER.sub(repl, template)


def expand_skill(
    skill: Skill,
    overrides: Optional[Mapping[str, Any]] = None,
) -> List[SkillStep]:
    """Return steps with ``{param}`` placeholders expanded."""
    ctx: Dict[str, Any] = {}
    ctx.update(default_parameters(skill))
    if overrides:
        ctx.update(dict(overrides))
    ctx.update(derived_variables(ctx))
    for k, v in skill.variables.items():
        ctx[k] = _expand_string(str(v), ctx)

    expanded: List[SkillStep] = []
    for step in skill.steps:
        new = SkillStep(
            id=step.id,
            title=step.title,
            type=step.type,
            sql=_expand_string(step.sql, ctx) if step.sql else None,
            method=step.method,
            path=_expand_string(step.path, ctx) if step.path else None,
            action=step.action,
            view=step.view,
            on_empty=step.on_empty,
            empty_message=step.empty_message,
            when=step.when,
            platform=step.platform,
            raw=step.raw,
        )
        expanded.append(new)
    return expanded


def _collect_keywords(skill: Skill) -> List[str]:
    words: List[str] = []
    words.extend(skill.tags)
    triggers = skill.triggers.get("keywords") or {}
    if isinstance(triggers, dict):
        for vals in triggers.values():
            if isinstance(vals, list):
                words.extend(str(v).lower() for v in vals)
    elif isinstance(triggers, list):
        words.extend(str(v).lower() for v in triggers)
    return words


def _match_intents(
    query: str, root: Optional[Path] = None, limit: int = 10
) -> List[str]:
    q = query.lower()
    data = load_intents(root)
    scored: List[tuple[int, str]] = []
    for intent in (data.get("intents") or {}).values():
        keywords = intent.get("keywords") or []
        hits = sum(1 for kw in keywords if str(kw).lower() in q)
        if hits:
            ids = intent.get("skills") or []
            for sid in ids:
                scored.append((hits, str(sid)))
    scored.sort(key=lambda x: (-x[0], x[1]))
    seen: set[str] = set()
    out: List[str] = []
    for _, sid in scored:
        if sid not in seen:
            seen.add(sid)
            out.append(sid)
        if len(out) >= limit:
            break
    return out


def match_skills(
    query: str,
    root: Optional[Path] = None,
    limit: int = 3,
) -> List[str]:
    """Rank skill ids by intent + keyword overlap with *query*."""
    q = query.lower()
    catalog = load_catalog(root)
    scores: Dict[str, int] = {}

    for rank, sid in enumerate(_match_intents(query, root, limit=10)):
        scores[sid] = scores.get(sid, 0) + 3 * (10 - rank)

    for entry in catalog.skills:
        skill = load_skill(entry.id, root)
        kw_score = sum(1 for kw in _collect_keywords(skill) if kw in q)
        if kw_score:
            scores[entry.id] = scores.get(entry.id, 0) + kw_score

    ranked = sorted(scores.items(), key=lambda x: (-x[1], x[0]))
    return [sid for sid, _ in ranked[:limit]]


def validate_skill(skill: Skill) -> List[str]:
    """Return a list of validation warnings (empty if ok)."""
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
        if step.type == "sql" and step.sql:
            upper = step.sql.strip().upper()
            if any(
                upper.startswith(k)
                for k in ("INSERT", "UPDATE", "DELETE", "DROP", "ALTER", "SET ")
            ):
                warnings.append(f"{skill.id}.{step.id}: sql step should be read-only")
            try:
                expand_skill(skill)
            except KeyError as e:
                warnings.append(f"{skill.id}.{step.id}: {e}")
    skill_md = skill.path.parent / "SKILL.md"
    if not skill_md.is_file():
        warnings.append(f"{skill.id}: missing SKILL.md")
    return warnings


def validate_all(root: Optional[Path] = None) -> List[str]:
    all_warnings: List[str] = []
    catalog = load_catalog(root)
    for entry in catalog.skills:
        skill = load_skill(entry.id, root)
        all_warnings.extend(validate_skill(skill))
    return all_warnings

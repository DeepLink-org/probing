"""Load and expand probing playbooks (YAML).

Requires PyYAML: ``pip install pyyaml`` (optional; only needed for playbook tooling).
"""

from __future__ import annotations

import os
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
            "Playbook loading requires PyYAML. Install with: pip install pyyaml"
        ) from e
    return yaml.safe_load(text)


def playbooks_root() -> Path:
    """Return the playbooks directory (repo root or PROBING_PLAYBOOKS_DIR)."""
    env = os.environ.get("PROBING_PLAYBOOKS_DIR")
    if env:
        return Path(env).expanduser().resolve()
    # python/probing/playbooks/loader.py -> repo root
    return Path(__file__).resolve().parents[3] / "playbooks"


@dataclass
class PlaybookStep:
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
class Playbook:
    id: str
    title: str
    category: str
    tags: List[str]
    triggers: Dict[str, Any]
    docs: str
    parameters: List[Dict[str, Any]]
    requires: Dict[str, Any]
    steps: List[PlaybookStep]
    interpretation: Dict[str, Any]
    summary_template: str
    next_steps: List[str]
    variables: Dict[str, str]
    path: Path
    metadata: Dict[str, Any] = field(default_factory=dict)


@dataclass
class PlaybookCatalogEntry:
    id: str
    file: str
    category: str
    priority: int
    description: str


@dataclass
class PlaybookCatalog:
    playbooks: List[PlaybookCatalogEntry]


def _parse_playbook(data: Mapping[str, Any], path: Path) -> Playbook:
    meta = data.get("metadata") or {}
    spec = data.get("spec") or {}
    steps: List[PlaybookStep] = []
    for raw in spec.get("steps") or []:
        steps.append(
            PlaybookStep(
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
    return Playbook(
        id=str(meta.get("id", path.stem)),
        title=str(meta.get("title", meta.get("id", path.stem))),
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


def load_catalog(root: Optional[Path] = None) -> PlaybookCatalog:
    root = root or playbooks_root()
    data = _yaml_load((root / "catalog.yaml").read_text(encoding="utf-8"))
    entries = [
        PlaybookCatalogEntry(
            id=str(p["id"]),
            file=str(p["file"]),
            category=str(p.get("category", "")),
            priority=int(p.get("priority", 0)),
            description=str(p.get("description", "")),
        )
        for p in data.get("playbooks") or []
    ]
    entries.sort(key=lambda e: e.priority)
    return PlaybookCatalog(playbooks=entries)


def load_playbook(playbook_id: str, root: Optional[Path] = None) -> Playbook:
    catalog = load_catalog(root)
    entry = next((p for p in catalog.playbooks if p.id == playbook_id), None)
    if entry is None:
        raise KeyError(f"Unknown playbook: {playbook_id}")
    root = root or playbooks_root()
    path = root / entry.file
    data = _yaml_load(path.read_text(encoding="utf-8"))
    return _parse_playbook(data, path)


def load_semantic_catalog(root: Optional[Path] = None) -> Dict[str, Any]:
    root = root or playbooks_root()
    path = root / "semantic" / "tables.yaml"
    return _yaml_load(path.read_text(encoding="utf-8"))


def default_parameters(playbook: Playbook) -> Dict[str, Any]:
    out: Dict[str, Any] = {}
    for p in playbook.parameters:
        name = p.get("name")
        if name is not None and "default" in p:
            out[str(name)] = p["default"]
    return out


def derived_variables(params: Mapping[str, Any]) -> Dict[str, str]:
    use_global = bool(params.get("use_global", False))
    comm = "global.python.comm_collective" if use_global else "python.comm_collective"
    return {
        "comm_table": comm,
        "table_comm": comm,
        "global_prefix": "global." if use_global else "",
    }


def _expand_string(template: str, ctx: Mapping[str, Any]) -> str:
    def repl(match: re.Match[str]) -> str:
        key = match.group(1)
        if key not in ctx:
            raise KeyError(f"Missing playbook parameter or variable: {key}")
        return str(ctx[key])

    return _PLACEHOLDER.sub(repl, template)


def expand_playbook(
    playbook: Playbook,
    overrides: Optional[Mapping[str, Any]] = None,
) -> List[PlaybookStep]:
    """Return steps with ``{param}`` placeholders expanded."""
    ctx: Dict[str, Any] = {}
    ctx.update(default_parameters(playbook))
    if overrides:
        ctx.update(dict(overrides))
    ctx.update(derived_variables(ctx))
    for k, v in playbook.variables.items():
        ctx[k] = _expand_string(str(v), ctx)

    expanded: List[PlaybookStep] = []
    for step in playbook.steps:
        new = PlaybookStep(
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


def _collect_keywords(playbook: Playbook) -> List[str]:
    words: List[str] = []
    words.extend(playbook.tags)
    triggers = playbook.triggers.get("keywords") or {}
    if isinstance(triggers, dict):
        for vals in triggers.values():
            if isinstance(vals, list):
                words.extend(str(v).lower() for v in vals)
    elif isinstance(triggers, list):
        words.extend(str(v).lower() for v in triggers)
    return words


def match_playbooks(
    query: str,
    root: Optional[Path] = None,
    limit: int = 3,
) -> List[str]:
    """Rank playbook ids by keyword overlap with *query* (for agent routing)."""
    q = query.lower()
    catalog = load_catalog(root)
    scored: List[tuple[int, str]] = []
    for entry in catalog.playbooks:
        pb = load_playbook(entry.id, root)
        score = sum(1 for kw in _collect_keywords(pb) if kw in q)
        if score:
            scored.append((score, entry.id))
    scored.sort(key=lambda x: (-x[0], x[1]))
    return [pid for _, pid in scored[:limit]]


def validate_playbook(playbook: Playbook) -> List[str]:
    """Return a list of validation warnings (empty if ok)."""
    warnings: List[str] = []
    if not playbook.steps:
        warnings.append(f"{playbook.id}: no steps defined")
    seen_ids: set[str] = set()
    for step in playbook.steps:
        if step.id in seen_ids:
            warnings.append(f"{playbook.id}: duplicate step id {step.id}")
        seen_ids.add(step.id)
        if step.type == "sql" and not step.sql:
            warnings.append(f"{playbook.id}.{step.id}: sql step missing sql")
        if step.type == "sql" and step.sql:
            upper = step.sql.strip().upper()
            if any(
                upper.startswith(k)
                for k in ("INSERT", "UPDATE", "DELETE", "DROP", "ALTER", "SET ")
            ):
                warnings.append(
                    f"{playbook.id}.{step.id}: sql step should be read-only"
                )
            # Check placeholders resolve with defaults
            try:
                expand_playbook(playbook)
            except KeyError as e:
                warnings.append(f"{playbook.id}.{step.id}: {e}")
    return warnings


def validate_all(root: Optional[Path] = None) -> List[str]:
    root = root or playbooks_root()
    all_warnings: List[str] = []
    catalog = load_catalog(root)
    for entry in catalog.playbooks:
        pb = load_playbook(entry.id, root)
        all_warnings.extend(validate_playbook(pb))
    return all_warnings

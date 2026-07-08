"""Low-level setuptools entry-point loading."""

from __future__ import annotations

import logging
from importlib import import_module
from pathlib import Path
from typing import Any, Callable, Dict, Iterable, List, Optional, Tuple, Type

logger = logging.getLogger(__name__)

ENTRY_GROUP_SKILLS = "probing.skills"
ENTRY_GROUP_MAGICS = "probing.magics"
_SKILLS_MARKER = "catalog.yaml"


def entry_points(group: str) -> Iterable[Any]:
    try:
        from importlib.metadata import entry_points as eps_fn
    except ImportError:
        from importlib_metadata import entry_points as eps_fn  # type: ignore

    try:
        selected = eps_fn(group=group)
    except TypeError:
        selected = eps_fn().get(group, [])
    return selected or []


def entry_point_names_for_dist(dist_name: str, group: str) -> List[str]:
    names: List[str] = []
    for ep in entry_points(group):
        ep_dist = getattr(ep, "dist", None)
        if ep_dist is None:
            continue
        if (ep_dist.metadata.get("Name") or ep_dist.name).lower() == dist_name.lower():
            names.append(ep.name)
    return sorted(set(names))


def _resolve_skill_root(loader: Callable[[], Any], label: str) -> Optional[Path]:
    try:
        target = loader()
    except Exception as exc:
        logger.warning("probing.skills entry point %s failed: %s", label, exc)
        return None
    if callable(target) and not isinstance(target, type):
        try:
            value = target()
        except Exception as exc:
            logger.warning("probing.skills entry point %s failed: %s", label, exc)
            return None
    else:
        value = target
    if value is None:
        return None
    path = value if isinstance(value, Path) else Path(str(value)).expanduser()
    try:
        path = path.resolve()
    except OSError:
        path = path.absolute()
    marker = path / _SKILLS_MARKER
    if not marker.is_file():
        logger.warning("probing.skills entry point %s: missing %s", label, marker)
        return None
    return path


def load_skill_roots_from_entry_points() -> List[Tuple[Path, str]]:
    """Skill directories registered through ``probing.skills`` entry points."""
    roots: List[Tuple[Path, str]] = []
    seen: set[str] = set()
    for ep in entry_points(ENTRY_GROUP_SKILLS):
        label = f"{ep.dist.name}:{ep.name}" if getattr(ep, "dist", None) else ep.name
        path = _resolve_skill_root(ep.load, label)
        if path is None:
            continue
        key = str(path)
        if key in seen:
            continue
        seen.add(key)
        roots.append((path, label))
    return roots


def load_magics_from_entry_points(registry: Dict[str, Type]) -> List[str]:
    """Load ``probing.magics`` entry points into *registry* (name → Magics class)."""
    loaded: List[str] = []
    for ep in entry_points(ENTRY_GROUP_MAGICS):
        name = ep.name
        try:
            target = ep.load()
        except Exception as exc:
            logger.warning("probing.magics entry point %s failed: %s", name, exc)
            continue
        if isinstance(target, type):
            cls = target
        else:
            module_name, _, attr = ep.value.partition(":")
            module = import_module(module_name)
            cls = getattr(module, attr or name, target)
        if name in registry:
            logger.warning("probing.magics: replacing existing magic %s", name)
        registry[name] = cls
        loaded.append(name)
    return loaded

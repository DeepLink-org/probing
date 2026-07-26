"""MetaPath finder that invents missing third-party packages."""

from __future__ import annotations

import importlib.abc
import importlib.machinery
import importlib.util
import logging
import sys
import types
from typing import Optional, Sequence

from . import registry

logger = logging.getLogger(__name__)

_FORCE = False


def set_force(force: bool) -> None:
    global _FORCE
    _FORCE = bool(force)


def force_enabled() -> bool:
    return _FORCE


class _FakeLoader(importlib.abc.Loader):
    def __init__(self, fullname: str, module: types.ModuleType):
        self.fullname = fullname
        self.module = module

    def create_module(self, spec: importlib.machinery.ModuleSpec) -> types.ModuleType:
        return self.module

    def exec_module(self, module: types.ModuleType) -> None:
        return None


class FakeFinder(importlib.abc.MetaPathFinder):
    """Invent modules for enabled fake specs when the real package is absent.

    With ``force=True`` (see ``set_force``), shadow even if a real package is
    installed — needed on macOS when megatron-core is present but unusable.
    """

    def find_spec(
        self,
        fullname: str,
        path: Optional[Sequence[str]],
        target: Optional[types.ModuleType] = None,
    ) -> Optional[importlib.machinery.ModuleSpec]:
        del path, target
        spec = registry.match_spec(fullname)
        if spec is None:
            return None
        if not _FORCE and _real_module_available(fullname):
            return None

        module = spec.factory(fullname)
        module.__name__ = fullname
        module.__package__ = fullname.rpartition(".")[0]
        module.__probing_fake__ = True  # type: ignore[attr-defined]
        if not hasattr(module, "__path__") and _looks_like_package(fullname, spec):
            module.__path__ = []  # type: ignore[attr-defined]
        loader = _FakeLoader(fullname, module)
        is_pkg = hasattr(module, "__path__")
        module_spec = importlib.util.spec_from_loader(
            fullname, loader, is_package=is_pkg
        )
        if module_spec is not None:
            module_spec.origin = f"probing.fakes:{spec.name}"
            module.__spec__ = module_spec
        note_injected(fullname)
        logger.debug(
            "probing.fakes serving %s via spec %s (force=%s)",
            fullname,
            spec.name,
            _FORCE,
        )
        return module_spec


_FINDER: FakeFinder | None = None
_INJECTED_MODULES: list[str] = []


def _looks_like_package(fullname: str, spec: registry.FakeSpec) -> bool:
    del spec
    return fullname.count(".") <= 1 or fullname.endswith((".core", ".training"))


def _real_module_available(fullname: str) -> bool:
    """True when a non-fake finder can resolve ``fullname``."""
    saved = list(sys.meta_path)
    try:
        sys.meta_path = [f for f in saved if not isinstance(f, FakeFinder)]
        try:
            found = importlib.util.find_spec(fullname)
        except (ImportError, ModuleNotFoundError, ValueError):
            return False
        return found is not None
    finally:
        sys.meta_path = saved


def purge_prefixed_modules(prefixes: Sequence[str]) -> list[str]:
    """Remove ``sys.modules`` entries under ``prefixes`` (for force reinstall)."""
    removed: list[str] = []
    for name in list(sys.modules):
        for prefix in prefixes:
            if name == prefix or name.startswith(prefix + "."):
                del sys.modules[name]
                removed.append(name)
                break
    return removed


def install_finder() -> FakeFinder:
    global _FINDER
    for existing in sys.meta_path:
        if isinstance(existing, FakeFinder):
            _FINDER = existing
            return existing
    finder = FakeFinder()
    sys.meta_path.insert(0, finder)
    _FINDER = finder
    return finder


def uninstall_finder() -> None:
    global _FINDER
    sys.meta_path[:] = [f for f in sys.meta_path if not isinstance(f, FakeFinder)]
    _FINDER = None
    set_force(False)


def note_injected(fullname: str) -> None:
    if fullname not in _INJECTED_MODULES:
        _INJECTED_MODULES.append(fullname)


def clear_injected_modules() -> None:
    """Drop modules previously served by probing.fakes from ``sys.modules``."""
    global _INJECTED_MODULES
    names = set(_INJECTED_MODULES)
    for name, mod in list(sys.modules.items()):
        if getattr(mod, "__probing_fake__", False):
            names.add(name)
        origin = getattr(getattr(mod, "__spec__", None), "origin", "") or ""
        if origin.startswith("probing.fakes:"):
            names.add(name)
    for name in sorted(names, key=lambda n: n.count("."), reverse=True):
        if name in sys.modules:
            del sys.modules[name]
    _INJECTED_MODULES = []

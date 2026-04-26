"""
Import hook：在指定模块被 import 时执行回调。

- register：模块名 -> 回调（或回调列表），loader 在模块加载后执行。
- add_module_callback(module_name, callback)：往 register 里追加回调，统一走这套机制。
"""

import importlib.abc
import importlib.util
import sys


# Lazy imports to avoid import errors if modules are not available
def _get_torch_init():
    """Lazy import of torch init function."""
    try:
        from probing.ext.torch import init as torch_init

        return torch_init
    except ImportError:
        return lambda: None


def _get_ray_init():
    """Lazy import of ray init function."""
    try:
        from probing.ext.ray import init as ray_init

        return ray_init
    except ImportError:
        return lambda: None


# Mapping from module names to callback functions
# Callbacks are called when the module is imported
# Use lazy loading to avoid import errors
register = {
    "torch": _get_torch_init(),
    "ray": _get_ray_init(),
}

# Record modules that have been triggered
triggered = {}


class ProbingLoader(importlib.abc.Loader):
    """Custom loader that executes callbacks immediately after module loading"""

    def __init__(self, original_loader, fullname):
        self.original_loader = original_loader
        self.fullname = fullname

    def create_module(self, spec):
        # Delegate module creation to the original loader
        return (
            self.original_loader.create_module(spec)
            if hasattr(self.original_loader, "create_module")
            else None
        )

    def exec_module(self, module):
        # First let the original loader execute the module
        if hasattr(self.original_loader, "exec_module"):
            self.original_loader.exec_module(module)
        elif hasattr(self.original_loader, "load_module"):
            self.original_loader.load_module(self.fullname)

        if self.fullname in register and self.fullname not in triggered:
            triggered[self.fullname] = True
            _run_callbacks(self.fullname)


class ProbingFinder(importlib.abc.MetaPathFinder):
    """Custom finder for intercepting module imports and wrapping loaders"""

    def __init__(self):
        # Store original finders to restore the import chain
        # Don't store self in original_meta_path
        self.original_meta_path = [
            f for f in sys.meta_path if not isinstance(f, ProbingFinder)
        ]

    def find_spec(self, fullname, path, target=None):
        # If not a module we're interested in, skip it
        if fullname not in register:
            return None

        # Avoid recursive calls
        if fullname in sys._ProbingFinder_in_progress:  # type: ignore
            return None

        sys._ProbingFinder_in_progress.add(fullname)  # type: ignore
        # Save current meta_path to restore later
        saved_meta_path = list(sys.meta_path)
        try:
            # Temporarily remove self to avoid recursion
            sys.meta_path = list(self.original_meta_path)

            # Use original finders to find the module
            spec = importlib.util.find_spec(fullname)

            # If module is found, wrap its loader
            if spec is not None and spec.loader is not None:
                loader = ProbingLoader(spec.loader, fullname)
                spec.loader = loader

            return spec
        finally:
            sys._ProbingFinder_in_progress.remove(fullname)  # type: ignore
            # Always restore meta_path to what it was before
            sys.meta_path = saved_meta_path


def _run_callbacks(module_name):
    """Run all callbacks for a module (no args, for loader compatibility)."""
    if module_name not in register:
        return
    callbacks = register[module_name]
    if isinstance(callbacks, list):
        for cb in callbacks:
            try:
                cb()
            except Exception as e:
                print(f"Error in callback for {module_name}: {e}")  # noqa: T201
    else:
        try:
            callbacks()
        except Exception as e:
            print(f"Error in callback for {module_name}: {e}")  # noqa: T201


def register_module_callback(module_name, callback):
    """Register callback function for module import (replaces any existing)."""
    register[module_name] = callback

    if module_name in sys.modules and module_name not in triggered:
        triggered[module_name] = True
        try:
            callback(sys.modules[module_name])
        except Exception as e:
            print(f"Error executing callback for {module_name}: {e}")  # noqa: T201


def add_module_callback(module_name, callback):
    """Add a callback for module import (keeps existing callbacks). Triggers when module is loaded."""
    if module_name not in register:
        register[module_name] = []
    if not isinstance(register[module_name], list):
        register[module_name] = [register[module_name]]
    register[module_name].append(callback)

    if module_name in sys.modules and module_name not in triggered:
        triggered[module_name] = True
        _run_callbacks(module_name)


# Initialize recursion protection set
if not hasattr(sys, "_ProbingFinder_in_progress"):
    sys._ProbingFinder_in_progress = set()  # type: ignore


# Install import hook
def install():
    # Ensure it's only installed once
    for finder in sys.meta_path:
        if isinstance(finder, ProbingFinder):
            return finder

    # Create and install the hook
    finder = ProbingFinder()
    sys.meta_path.insert(0, finder)
    return finder


# Automatically install the hook
finder = install()

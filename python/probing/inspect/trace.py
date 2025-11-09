import ctypes
import fnmatch
import functools
import inspect
import json
import os
import sys
import threading
import warnings
from types import FrameType, FunctionType, ModuleType
from typing import Any, AnyStr, Callable, Dict, List, Set, Optional

thread_global = threading.local()
internal_directories = os.path.dirname((lambda: 0).__code__.co_filename)

traced_functions = {}


class _TraceableCollector:
    """Internal helper class for collecting traceable items from modules.

    This class encapsulates all the logic for discovering and filtering
    traceable functions and modules. It's not meant to be used directly
    by external code.
    """

    # Class-level constants
    WHITELIST = ["__main__"]
    BLACKLIST = ["numpy", "typing", "typing.io", "typing_extensions"]

    @staticmethod
    def create_filter(prefix: Optional[str]) -> Callable[[str], bool]:
        """Create a filter function based on the prefix pattern.

        Args:
            prefix: String prefix, can contain wildcards (* and ?)

        Returns:
            A filter function that returns True if a path matches the pattern
        """
        if prefix is None:
            return lambda x: True
        elif "*" in prefix or "?" in prefix:
            return lambda x: fnmatch.fnmatch(x, prefix)
        else:
            return lambda x: x.startswith(prefix)

    @staticmethod
    def get_object_name(obj: Any) -> Optional[str]:
        """Get the name of an object, with exception handling.

        Some objects may raise exceptions when accessing __name__ attribute.

        Args:
            obj: The object to get the name from

        Returns:
            The object's __name__ if it's a string, None otherwise
        """
        try:
            if hasattr(obj, "__name__"):
                with warnings.catch_warnings():
                    warnings.filterwarnings(
                        "ignore",
                        category=FutureWarning,
                        message=".*torch.distributed.reduce_op.*",
                    )
                    name = obj.__name__
                    if isinstance(name, str):
                        return name
        except (AttributeError, TypeError, RuntimeError):
            pass
        return None

    @staticmethod
    def should_skip_prefix(prefix: str, blacklist: List[str]) -> bool:
        """Check if a prefix should be skipped based on blacklist and special rules.
        
        Args:
            prefix: The prefix string to check
            blacklist: List of blacklisted prefixes
            
        Returns:
            True if the prefix should be skipped, False otherwise
        """
        if prefix in blacklist:
            return True
        
        # Special handling for torch module (but not torchvision, torchaudio, etc.)
        if prefix == "torch" or prefix.startswith("torch."):
            allowed_prefixes = ("torch.nn", "torch.cuda", "torch.distributed", "torch.optim")
            if not any(prefix.startswith(p) for p in allowed_prefixes):
                return True
        
        # Skip six module internals
        if prefix.startswith("six."):
            return True
        
        return False

    @staticmethod
    def determine_item_type(obj: Any) -> str:
        """Determine the type of an object (Function, Class, Module, or Variable).

        Args:
            obj: The object to classify

        Returns:
            A single character string: "F", "C", "M", or "V"
        """
        if inspect.isfunction(obj) or (
            isinstance(obj, FunctionType) and hasattr(obj, "__code__")
        ):
            return "F"
        elif inspect.isclass(obj):
            return "C"
        elif isinstance(obj, ModuleType):
            return "M"
        else:
            return "V"

    @staticmethod
    def should_include_module(
        module_name: str, module: ModuleType, whitelist: List[str]
    ) -> bool:
        """Check if a module should be included in the traceable items.

        Args:
            module_name: Name of the module
            module: The module object
            whitelist: List of whitelisted module names

        Returns:
            True if the module should be included, False otherwise
        """
        # Handle whitelist modules
        if module_name in whitelist:
            return True

        # Allow probing module
        if module_name.startswith("probing"):
            return isinstance(module_name, str) and not module_name.startswith("__")

        # For other modules, check __spec__ and site-packages
        if not hasattr(module, "__spec__"):
            return False

        try:
            if module.__spec__ is None or "site-packages" not in module.__spec__.origin:
                return False
        except (AttributeError, TypeError):
            return False

        return isinstance(module_name, str) and not module_name.startswith("__")

    @classmethod
    def traverse_object(
        cls,
        obj: Any,
        prefix: str,
        current_depth: int,
        depth: int,
        blacklist: List[str],
        filter_func: Callable[[str], bool],
        traceable_items: List[Dict],
        travel_history: Set[int],
    ) -> None:
        """Recursively traverse an object to find traceable functions.

        Args:
            obj: The object to traverse
            prefix: Current path prefix
            current_depth: Current recursion depth
            depth: Maximum recursion depth
            blacklist: List of blacklisted prefixes
            filter_func: Function to filter paths
            traceable_items: List to append found items to
            travel_history: Set of already visited object IDs
        """
        try:
            if id(obj) in travel_history or cls.should_skip_prefix(prefix, blacklist):
                return
            if current_depth > depth:
                return

            travel_history.add(id(obj))

            if not hasattr(obj, "__dict__"):
                return

            try:
                for k, v in obj.__dict__.items():
                    try:
                        if k.startswith("__"):
                            continue

                        full_path = f"{prefix}.{k}" if prefix else k
                        item_type = cls.determine_item_type(v)

                        if filter_func(full_path):
                            if item_type == "F":
                                traceable_items.append(
                                    {"name": full_path, "type": item_type}
                                )
                            elif not isinstance(v, ModuleType):
                                name = cls.get_object_name(v)
                                if name is not None and not name.startswith("__"):
                                    cls.traverse_object(
                                        v,
                                        full_path,
                                        current_depth + 1,
                                        depth,
                                        blacklist,
                                        filter_func,
                                        traceable_items,
                                        travel_history,
                                    )
                            else:
                                traceable_items.append(
                                    {"name": full_path, "type": item_type}
                                )

                    except (AttributeError, TypeError, RuntimeError, ValueError):
                        continue
            except (AttributeError, TypeError, RuntimeError):
                pass
        except (AttributeError, TypeError, RuntimeError, ValueError):
            pass

    @classmethod
    def collect_traceable_items(
        cls, depth: int, filter_func: Callable[[str], bool]
    ) -> List[Dict]:
        """Collect all traceable items from sys.modules.

        Args:
            depth: Maximum recursion depth for traversal
            filter_func: Function to filter paths

        Returns:
            List of traceable items (dicts with 'name' and 'type' keys)
        """
        traceable_items = []
        travel_history = set()

        # Check __main__ module first
        if "__main__" in sys.modules:
            main_module = sys.modules["__main__"]
            if isinstance(main_module, ModuleType):
                cls.traverse_object(
                    main_module,
                    "__main__",
                    0,
                    depth,
                    cls.BLACKLIST,
                    filter_func,
                    traceable_items,
                    travel_history,
                )

        # Traverse other modules
        for module_name, module in sys.modules.items():
            try:
                if not isinstance(module, ModuleType):
                    continue

                if module_name == "__main__":
                    continue

                if cls.should_include_module(module_name, module, cls.WHITELIST):
                    cls.traverse_object(
                        module,
                        module_name,
                        0,
                        depth,
                        cls.BLACKLIST,
                        filter_func,
                        traceable_items,
                        travel_history,
                    )
            except (AttributeError, TypeError, RuntimeError, ValueError):
                continue

        return traceable_items

    @staticmethod
    def filter_by_prefix(
        traceable_items: List[Dict], prefix: Optional[str]
    ) -> List[Dict]:
        """Filter and group traceable items based on prefix.

        Args:
            traceable_items: List of items with 'name' and 'type' keys
            prefix: Prefix to filter by (can be None or contain wildcards)

        Returns:
            Filtered and grouped list of traceable items
        """
        if prefix is None:
            # Return only top-level modules
            module_paths = {}
            for item in traceable_items:
                func_path = item["name"]
                parts = func_path.split(".")
                if len(parts) > 0:
                    top_level = parts[0]
                    if top_level not in module_paths:
                        module_paths[top_level] = {"name": top_level, "type": "M"}
            return sorted(module_paths.values(), key=lambda x: x["name"])

        has_wildcard = "*" in prefix or "?" in prefix
        if has_wildcard:
            # Return all matching paths without truncation
            return sorted(traceable_items, key=lambda x: x["name"])

        # Return paths one level deeper than prefix
        prefix_levels = len(prefix.split("."))
        module_paths = {}

        for item in traceable_items:
            func_path = item["name"]
            if not func_path.startswith(prefix):
                continue

            parts = func_path.split(".")
            if len(parts) > prefix_levels:
                module_path = ".".join(parts[: prefix_levels + 1])
                if module_path not in module_paths:
                    item_type = (
                        "M" if func_path.startswith(module_path + ".") else item["type"]
                    )
                    module_paths[module_path] = {"name": module_path, "type": item_type}
            elif len(parts) == prefix_levels and func_path == prefix:
                pass
            else:
                if func_path not in module_paths:
                    module_paths[func_path] = item

        return sorted(module_paths.values(), key=lambda x: x["name"])


def probe(func=None, watch=[], depth=1):
    if func is not None:

        @functools.wraps(func)
        def wrapper(*args, **kwargs):
            tracer = ProbingTracer(depth, watch)
            with tracer:
                return func(*args, **kwargs)

        return wrapper
    else:

        def decorator(func):
            @functools.wraps(func)
            def wrapper(*args, **kwargs):
                tracer = ProbingTracer(depth, watch)
                with tracer:
                    return func(*args, **kwargs)

            return wrapper

        return decorator


class ProbingTracer:
    def __init__(self, depth=1, watch=[]):
        self.depth = depth
        self.count_calls = 0
        self.count_returns = 0
        self.watch = watch
        self.watch_impl = {}

    def on_call(self):
        self.count_calls += 1

    def on_return(self):
        self.count_returns += 1

    def _outof_depth(self):
        depth = self.count_calls - self.count_returns
        return depth > self.depth

    def _is_internal_frame(self, frame):
        return frame.f_code.co_filename.startswith(internal_directories)

    def __enter__(self):
        tracer_stack = thread_global.__dict__.setdefault("tracer_stack", [])
        tracer_stack.append(sys.gettrace())
        sys.settrace(self.trace)
        return self

    def __exit__(self, exc_type, exc_val, exc_tb):
        tracer_stack = thread_global.tracer_stack
        sys.settrace(tracer_stack.pop())

    def trace(self, frame: FrameType, event: AnyStr, arg: Any):
        import torch

        # print(
        #     f"Event: {event}, Frame: {frame}, Arg: {arg}, name: {frame.f_code.co_name}"
        # )

        if event == "call":
            self.on_call()
            if self._outof_depth():
                frame.f_locals["__trace_checkpoint__"] = TracerCheckpoint(
                    self.on_return
                )
                return None
            if not self.watch_impl and self.watch:
                self.watch_impl = {
                    k: id(frame.f_locals.get(k, None)) for k in self.watch
                }
                print(
                    f"In {frame.f_code.co_name} from {frame.f_code.co_filename} line {frame.f_lineno}:"
                )
                print(f"  start watching variables: {[k for k in self.watch_impl]}")
            return self.trace
        if event == "return":
            self.on_return()
            for k in self.watch:
                if k in frame.f_locals and isinstance(
                    frame.f_locals[k], FakeProbingTensor
                ):
                    frame.f_locals[k] = torch.Tensor(frame.f_locals[k])
                    ctypes.pythonapi.PyFrame_LocalsToFast(
                        ctypes.py_object(frame), ctypes.c_int(0)
                    )
            return self.trace
        if self._is_internal_frame(frame):
            return None

        for k in self.watch:
            if (
                k in frame.f_locals
                and isinstance(frame.f_locals[k], torch.Tensor)
                and (not isinstance(frame.f_locals[k], FakeProbingTensor))
            ):
                frame.f_locals[k] = ProbingTensor(frame.f_locals[k])
                ctypes.pythonapi.PyFrame_LocalsToFast(
                    ctypes.py_object(frame), ctypes.c_int(0)
                )
        for k, v in self.watch_impl.items():
            if k in frame.f_locals and id(frame.f_locals[k]) != v:
                print(f"probing: variable update {k} = {frame.f_locals[k]}")
                self.watch_impl[k] = id(frame.f_locals[k])
        return self.trace


class TracerCheckpoint:
    def __init__(self, callback=None):
        self.trace = sys.gettrace()
        self.callback = callback
        sys.settrace(None)

    def __del__(self):
        if self.callback:
            self.callback()
        sys.settrace(self.trace)


class FakeProbingTensor:
    pass


__ProbingTensor = None


def ProbingTensor(*args, **kwargs):
    import torch

    class _ProbingTensor(torch.Tensor, FakeProbingTensor):
        def __format__(self, format_spec):
            return f"{self.item().__format__(format_spec)}"

        @classmethod
        def __torch_function__(cls, func, types, args=(), kwargs=None):
            if kwargs is None:
                kwargs = {}
            if (
                func is not torch.Tensor.__repr__
                # and func is not torch.Tensor.__format__
                and func is not torch.Tensor.__str__
                and func.__name__.endswith("_")
                and not func.__name__.startswith("__")
            ):
                old_val = f"{args}"
                ret = super().__torch_function__(func, types, args, kwargs)
                ret_val = f"{args}"
                print(
                    f"probing: tensor update with {func.__name__}: {old_val} => {ret_val}"
                )
                return ret
            return super().__torch_function__(func, types, args, kwargs)

    global __ProbingTensor
    if __ProbingTensor is None:
        __ProbingTensor = _ProbingTensor
    return __ProbingTensor(*args, **kwargs)


def list_traceable(prefix=None, depth=2):
    """List all traceable functions and modules.

    Public API for discovering traceable functions and modules in the Python environment.
    Supports wildcard patterns (* and ?) for flexible filtering.

    Args:
        prefix: Optional prefix to filter results. Supports wildcards:
                - None: Returns top-level modules only
                - "torch.nn": Returns items one level deeper (torch.nn.Linear, torch.nn.Module, etc.)
                - "torch.*": Returns all items matching the pattern
        depth: Maximum depth for recursive traversal (default: 2)

    Returns:
        JSON string containing list of formatted traceable items in "[TYPE] name" format
        where TYPE is F (Function), C (Class), M (Module), or V (Variable)

    Examples:
        >>> list_traceable()  # Returns top-level modules
        >>> list_traceable("torch.nn")  # Returns torch.nn.* items
        >>> list_traceable("torch.*.Linear")  # Returns all Linear classes in torch submodules
    """
    collector = _TraceableCollector()
    filter_func = collector.create_filter(prefix)
    traceable_items = collector.collect_traceable_items(depth, filter_func)
    traceable_items = collector.filter_by_prefix(traceable_items, prefix)

    # Format items as "[TYPE] name"
    formatted_items = [f"[{x['type']}] {x['name']}" for x in traceable_items]
    return json.dumps(formatted_items, indent=2)


def getname(obj):
    """Get the name of an object.

    Public API for getting object names with proper exception handling.

    Args:
        obj: Any Python object

    Returns:
        The object's __name__ attribute if it exists and is a string, None otherwise
    """
    return _TraceableCollector.get_object_name(obj)


def trace(func_or_name, watch=[], depth=1, callback=None):
    def get_func(name):
        names = name.split(".")
        parent = sys.modules.get(names[0], None)
        names = names[1:]
        while parent is not None and len(names) > 0:
            if hasattr(parent, names[0]):
                if len(names) == 1:
                    return parent, getattr(parent, names[0]), names[0]
                parent = getattr(parent, names[0])
                names = names[1:]
            else:
                raise ValueError(f"{names[0]} not found in {parent}.")

    if isinstance(func_or_name, str):
        if func_or_name in traced_functions:
            print(f"Function {func_or_name} is already being traced.")
            return
        try:
            parent, func, name = get_func(func_or_name)
            traced_functions[func_or_name] = func
            func = probe(func, watch=watch, depth=depth)
            parent.__setattr__(name, func)
        except Exception:
            print(f"Function {func_or_name} not found.")
            return
    else:
        raise NotImplementedError("Only string names are supported for tracing.")


def untrace(func_or_name):
    def get_func(name):
        names = name.split(".")
        parent = sys.modules.get(names[0], None)
        names = names[1:]
        while parent is not None and len(names) > 0:
            if hasattr(parent, names[0]):
                if len(names) == 1:
                    return parent, getattr(parent, names[0]), names[0]
                parent = getattr(parent, names[0])
                names = names[1:]
            else:
                raise ValueError(f"{names[0]} not found in {parent}.")

    if isinstance(func_or_name, str):
        if func_or_name not in traced_functions:
            print(f"Function {func_or_name} is not being traced.")
            return
        try:
            parent, func, name = get_func(func_or_name)
            func = traced_functions.pop(func_or_name)
            parent.__setattr__(name, func)
        except Exception:
            print(f"Function {func_or_name} not found.")
            return
    else:
        raise NotImplementedError("Only string names are supported for tracing.")


def show_trace():
    return json.dumps([x for x in traced_functions.keys()], indent=2)

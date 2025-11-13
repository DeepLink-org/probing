import ctypes
import fnmatch
import functools
import inspect
import json
import os
import sys
import threading
import time
import types
import warnings
from dataclasses import dataclass
from types import FrameType, FunctionType, ModuleType
from typing import Any, AnyStr, Callable, Dict, List, Set, Optional

from probing.core.table import table

thread_global = threading.local()
internal_directories = os.path.dirname((lambda: 0).__code__.co_filename)

traced_functions = {}
# Global dictionary to store probe attributes for functions
# Key: function code object id, Value: dict with __probe_func__, __probe_watch__, __probe_depth__
_probe_attrs = {}
# Global dictionary to store module references needed by wrapper functions
_probe_modules = {'sys': sys}


@table("trace_variables")
@dataclass
class Variable:
    """Row model for variable change records.
    
    Each saved instance represents a variable change during function tracing.
    
    Parameters
    ----------
    function_name : str
        Name of the function where the variable change occurred.
    filename : str
        Name of the file containing the function.
    lineno : int
        Line number where the variable change occurred.
    variable_name : str
        Name of the variable that changed.
    value : str
        String representation of the variable value.
    value_type : str
        Type name of the variable value.
    """
    function_name: str
    filename: str
    lineno: int
    variable_name: str
    value: str
    value_type: str


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
                                # Get function variables (parameters and local variables)
                                variables = []
                                try:
                                    if isinstance(v, FunctionType) and hasattr(v, "__code__"):
                                        code = v.__code__
                                        
                                        # Use co_varnames to get all local variables (including parameters)
                                        # co_varnames contains: parameters, *args, **kwargs, and local variables
                                        # Use co_nlocals to get the count of all local names
                                        try:
                                            if hasattr(code, "co_varnames") and hasattr(code, "co_nlocals"):
                                                varnames = getattr(code, "co_varnames", ())
                                                nlocals = getattr(code, "co_nlocals", 0)
                                                
                                                if varnames and nlocals > 0:
                                                    # Get all local variables (parameters + locals)
                                                    # co_varnames[:co_nlocals] gives us all local names
                                                    variables = list(varnames[:nlocals])
                                                    
                                                    # Also include co_names for referenced globals (optional)
                                                    # This helps identify what external names the function uses
                                                    if hasattr(code, "co_names"):
                                                        names = getattr(code, "co_names", ())
                                                        if names:
                                                            # Filter out builtins and common functions
                                                            filtered_names = [
                                                                name for name in names 
                                                                if not name.startswith("__") 
                                                                and name not in ["print", "len", "str", "int", "float", "list", "dict", "tuple", "set", "range", "enumerate", "zip"]
                                                            ]
                                                            # Add to variables list (these are globals used in the function)
                                                            variables.extend(filtered_names)
                                                    
                                                    # Remove duplicates and sort
                                                    variables = sorted(list(set(variables)))
                                                    
                                        except (AttributeError, TypeError, IndexError):
                                            pass
                                        
                                        # Fallback: try to get just parameters if co_nlocals approach failed
                                        if not variables:
                                            try:
                                                if hasattr(code, "co_varnames") and hasattr(code, "co_argcount"):
                                                    argcount = getattr(code, "co_argcount", 0)
                                                    kwonlyargcount = getattr(code, "co_kwonlyargcount", 0)
                                                    posonlyargcount = getattr(code, "co_posonlyargcount", 0)
                                                    varnames = getattr(code, "co_varnames", ())
                                                    if varnames:
                                                        # Get parameters only
                                                        param_count = argcount + posonlyargcount + kwonlyargcount
                                                        if param_count > 0:
                                                            variables = list(varnames[:param_count])
                                            except (AttributeError, TypeError, IndexError):
                                                pass
                                        
                                        # Final fallback: use inspect.signature
                                        if not variables:
                                            try:
                                                if hasattr(v, "__name__") and not v.__name__.startswith("_"):
                                                    sig = inspect.signature(v, follow_wrapped=False)
                                                    variables = [param.name for param in sig.parameters.values()]
                                            except (ValueError, TypeError, AttributeError, RuntimeError):
                                                pass
                                except (AttributeError, TypeError, RuntimeError, ValueError):
                                    # Skip this function if any error occurs
                                    pass
                                traceable_items.append(
                                    {"name": full_path, "type": item_type, "variables": variables}
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
                    # Preserve all fields from original item, especially 'variables'
                    new_item = item.copy()
                    new_item["name"] = module_path
                    new_item["type"] = item_type
                    # For modules, clear variables if it was a function
                    if item_type == "M":
                        new_item["variables"] = []
                    module_paths[module_path] = new_item
            elif len(parts) == prefix_levels and func_path == prefix:
                # Exact match: include the item itself (important for getting variables of a specific function)
                if func_path not in module_paths:
                    module_paths[func_path] = item.copy()
            else:
                if func_path not in module_paths:
                    module_paths[func_path] = item.copy()

        return sorted(module_paths.values(), key=lambda x: x["name"])


def probe(func, watch=None, silent_watch=None, depth=1):
    """Wrap a function with tracing capabilities.
    
    Args:
        func: Function to wrap
        watch: List of variable names to watch and print (default: [])
        silent_watch: List of variable names to watch but only log to table (default: [])
        depth: Tracing depth (default: 1)
    
    Returns:
        Wrapped function that traces execution
    """
    if watch is None:
        watch = []
    if silent_watch is None:
        silent_watch = []
    
    def wrapper(*args, **kwargs):
        # Get code object id from current frame
        _sys_module = __import__('sys')
        current_frame = _sys_module._getframe(0)
        code_id = id(current_frame.f_code)
        
        # Get _probe_attrs from trace module (not from current frame's globals)
        _trace_module = _sys_module.modules.get('probing.inspect.trace')
        if _trace_module is None:
            for mod in _sys_module.modules.values():
                if hasattr(mod, '_probe_attrs'):
                    _trace_module = mod
                    break
        if _trace_module is None:
            raise RuntimeError("Cannot find trace module with _probe_attrs")
        
        _probe_attrs_dict = getattr(_trace_module, '_probe_attrs', {})
        attrs = _probe_attrs_dict.get(code_id, {})
        _func = attrs.get('__probe_func__')
        if _func is None:
            raise RuntimeError(f"Probe attributes not found for code id {code_id}")
        
        ProbingTracer = getattr(_trace_module, 'ProbingTracer')
        tracer = ProbingTracer(
            attrs.get('__probe_depth__', 1), 
            attrs.get('__probe_watch__', []),
            attrs.get('__probe_silent_watch__', [])
        )
        with tracer:
            return _func(*args, **kwargs)
    
    # Store attributes in global dict keyed by code object id
    code_id = id(wrapper.__code__)
    _probe_attrs[code_id] = {
        '__probe_func__': func,
        '__probe_watch__': watch if isinstance(watch, list) else list(watch) if watch else [],
        '__probe_silent_watch__': silent_watch if isinstance(silent_watch, list) else list(silent_watch) if silent_watch else [],
        '__probe_depth__': depth,
    }
    
    wrapper.__globals__['_probe_attrs'] = _probe_attrs
    wrapper.__globals__['_probe_modules'] = _probe_modules
    
    # Apply functools.wraps to copy metadata
    wrapper = functools.wraps(func)(wrapper)
    
    # Re-store attributes after functools.wraps (may have created new function with new code)
    code_id = id(wrapper.__code__)
    _probe_attrs[code_id] = {
        '__probe_func__': func,
        '__probe_watch__': watch if isinstance(watch, list) else list(watch) if watch else [],
        '__probe_silent_watch__': silent_watch if isinstance(silent_watch, list) else list(silent_watch) if silent_watch else [],
        '__probe_depth__': depth,
    }
    wrapper.__globals__['_probe_attrs'] = _probe_attrs
    wrapper.__globals__['_probe_modules'] = _probe_modules
    
    return wrapper


class ProbingTracer:
    def __init__(self, depth=1, watch=[], silent_watch=[]):
        self.depth = depth
        self.count_calls = 0
        self.count_returns = 0
        self.watch = watch  # Variables to watch and print
        self.silent_watch = silent_watch  # Variables to watch but only log to table
        # Combined list of all watched variables
        self.all_watch = list(set(watch + silent_watch))
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
            if not self.watch_impl and self.all_watch:
                self.watch_impl = {
                    k: id(frame.f_locals.get(k, None)) for k in self.all_watch
                }
            return self.trace
        if event == "return":
            self.on_return()
            for k in self.all_watch:
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

        for k in self.all_watch:
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
                new_value = frame.f_locals[k]
                # Format: probing: [function:line] variable = value (type)
                filename = frame.f_code.co_filename.split('/')[-1] if '/' in frame.f_code.co_filename else frame.f_code.co_filename
                value_str = str(new_value)
                value_type = type(new_value).__name__
                
                # Print only if variable is in watch list (not silent_watch)
                if k in self.watch:
                    print(f"probing: variable update {k} = {new_value}")
                
                # Save variable change to trace_variables table (for both watch and silent_watch)
                try:
                    # Get full qualified function name (module.function_name)
                    module_name = frame.f_globals.get('__name__', '')
                    func_name = frame.f_code.co_name
                    if module_name:
                        full_function_name = f"{module_name}.{func_name}"
                    else:
                        full_function_name = func_name
                    
                    Variable(
                        function_name=full_function_name,
                        filename=filename,
                        lineno=frame.f_lineno,
                        variable_name=k,
                        value=value_str,
                        value_type=value_type,
                    ).save()
                except Exception as e:
                    # Log error but don't disrupt the tracing process
                    print(f"Warning: Failed to save variable change to trace_variables table: {e}")
                
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
    Always returns structured data with variables information.

    Args:
        prefix: Optional prefix to filter results. Supports wildcards.
            Examples: None (returns top-level modules),
            torch.nn (returns torch.nn.* items),
            torch.* (returns all matching items).
        depth: Maximum depth for recursive traversal (default: 2)

    Returns:
        JSON string containing list of items. Each item is a dict with "name", "type", and "variables" keys.
        Functions (type='F') will have their variables (parameters and locals) in the "variables" list.
        Other items (modules, classes, etc.) will have an empty "variables" list.

    Examples:
        >>> list_traceable()  # doctest: +SKIP
        >>> list_traceable("torch.nn")  # doctest: +SKIP
        >>> list_traceable("torch.*.Linear")  # doctest: +SKIP
    """
    collector = _TraceableCollector()
    filter_func = collector.create_filter(prefix)
    traceable_items = collector.collect_traceable_items(depth, filter_func)
    traceable_items = collector.filter_by_prefix(traceable_items, prefix)

    # Always return structured data with variables
    result = []
    for item in traceable_items:
        result.append({
            "name": item['name'],
            "type": item['type'],
            "variables": item.get('variables', [])
        })
    return json.dumps(result, indent=2)


def getname(obj):
    """Get the name of an object.

    Public API for getting object names with proper exception handling.

    Args:
        obj: Any Python object

    Returns:
        The object's __name__ attribute if it exists and is a string, None otherwise
    """
    return _TraceableCollector.get_object_name(obj)


def trace(func_or_name, watch=[], silent_watch=[], depth=1, callback=None):
    def get_func(name):
        names = name.split(".")
        parent = sys.modules.get(names[0], None)
        names = names[1:]
        while parent is not None and len(names) > 0:
            if hasattr(parent, names[0]):
                if len(names) == 1:
                    return getattr(parent, names[0])
                parent = getattr(parent, names[0])
                names = names[1:]
            else:
                raise ValueError(f"{names[0]} not found in {parent}.")

    if isinstance(func_or_name, str):
        if func_or_name in traced_functions:
            print(f"Function {func_or_name} is already being traced.")
            return
        try:
            func = get_func(func_or_name)
            if not isinstance(func, FunctionType):
                print(f"Error: {func_or_name} is not a function")
                return
            
            # Store original attributes for restoration
            original_attrs = {
                '__code__': func.__code__,
                '__defaults__': func.__defaults__,
                '__kwdefaults__': func.__kwdefaults__.copy() if func.__kwdefaults__ else None,
                '__closure__': func.__closure__,
                '__code_id__': id(func.__code__),  # Store original code_id for cleanup
            }
            traced_functions[func_or_name] = original_attrs
            
            # Create a copy of the original function to avoid recursion
            # When we replace func.__code__, the func object itself is modified
            # So we need to store a copy that won't be affected
            original_func = types.FunctionType(
                func.__code__,
                func.__globals__,
                func.__name__,
                func.__defaults__,
                func.__closure__
            )
            original_func.__kwdefaults__ = func.__kwdefaults__.copy() if func.__kwdefaults__ else None
            original_func.__annotations__ = getattr(func, '__annotations__', None)
            original_func.__doc__ = func.__doc__
            original_func.__module__ = getattr(func, '__module__', None)
            
            # Create wrapped function using probe with the original function copy
            wrapped_func = probe(original_func, watch=watch, silent_watch=silent_watch, depth=depth)
            
            # Create a new function object using types.FunctionType to ensure proper validation
            wrapper_globals = wrapped_func.__globals__.copy()
            wrapper_globals['_probe_attrs'] = _probe_attrs
            wrapper_globals['_probe_modules'] = _probe_modules
            new_func = types.FunctionType(
                wrapped_func.__code__,
                wrapper_globals,
                func.__name__,
                wrapped_func.__defaults__,
                wrapped_func.__closure__
            )
            
            # Copy additional attributes
            new_func.__kwdefaults__ = wrapped_func.__kwdefaults__
            new_func.__annotations__ = getattr(func, '__annotations__', None)
            new_func.__doc__ = func.__doc__
            new_func.__module__ = getattr(func, '__module__', None)
            
            # Update _probe_attrs before replacing __code__ (new code object will have different id)
            wrapped_code_id = id(wrapped_func.__code__)
            new_code_id = id(new_func.__code__)
            if wrapped_code_id in _probe_attrs:
                _probe_attrs[new_code_id] = _probe_attrs[wrapped_code_id].copy()
            
            # Replace function attributes
            try:
                func.__code__ = new_func.__code__
                func.__defaults__ = new_func.__defaults__
                func.__kwdefaults__ = new_func.__kwdefaults__
                func.__closure__ = new_func.__closure__
            except (AttributeError, TypeError):
                # If direct assignment fails (readonly attribute), use object.__setattr__
                object.__setattr__(func, '__code__', new_func.__code__)
                object.__setattr__(func, '__defaults__', new_func.__defaults__)
                object.__setattr__(func, '__kwdefaults__', new_func.__kwdefaults__)
                object.__setattr__(func, '__closure__', new_func.__closure__)
            
            # Ensure final code_id is in _probe_attrs
            final_code_id = id(func.__code__)
            if final_code_id not in _probe_attrs and new_code_id in _probe_attrs:
                _probe_attrs[final_code_id] = _probe_attrs[new_code_id].copy()
            
            # Store all code_ids associated with this function for cleanup in untrace
            # Collect all code_ids that might be in _probe_attrs
            all_traced_code_ids = []
            if wrapped_code_id in _probe_attrs:
                all_traced_code_ids.append(wrapped_code_id)
            if new_code_id in _probe_attrs:
                all_traced_code_ids.append(new_code_id)
            if final_code_id in _probe_attrs:
                all_traced_code_ids.append(final_code_id)
            # Also include the code_id from probe function (wrapper's code_id before functools.wraps)
            # This is stored in probe function, we need to get it from wrapped_func
            # Actually, wrapped_code_id should already be the one from probe
            traced_functions[func_or_name]['__traced_code_ids__'] = list(set(all_traced_code_ids))
            
        except Exception as e:
            print(f"Function {func_or_name} not found: {e}")
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
                    return getattr(parent, names[0])
                parent = getattr(parent, names[0])
                names = names[1:]
            else:
                raise ValueError(f"{names[0]} not found in {parent}.")

    if isinstance(func_or_name, str):
        if func_or_name not in traced_functions:
            print(f"Function {func_or_name} is not being traced.")
            return
        try:
            func = get_func(func_or_name)
            if not isinstance(func, FunctionType):
                print(f"Error: {func_or_name} is not a function")
                return
            
            # Get original attributes
            original_attrs = traced_functions.pop(func_or_name)
            
            # Clean up _probe_attrs entries for all code_ids associated with this function
            traced_code_ids = original_attrs.get('__traced_code_ids__', [])
            current_code_id = id(func.__code__)
            # Also check current code_id in case it's different
            all_code_ids = set(traced_code_ids + [current_code_id])
            for code_id in all_code_ids:
                if code_id in _probe_attrs:
                    del _probe_attrs[code_id]
            
            # Restore function's attributes
            try:
                func.__code__ = original_attrs['__code__']
                func.__defaults__ = original_attrs['__defaults__']
                func.__kwdefaults__ = original_attrs['__kwdefaults__']
                func.__closure__ = original_attrs['__closure__']
            except (AttributeError, TypeError):
                # If direct assignment fails (readonly attribute), use object.__setattr__
                object.__setattr__(func, '__code__', original_attrs['__code__'])
                object.__setattr__(func, '__defaults__', original_attrs['__defaults__'])
                object.__setattr__(func, '__kwdefaults__', original_attrs['__kwdefaults__'])
                object.__setattr__(func, '__closure__', original_attrs['__closure__'])
            
        except Exception as e:
            print(f"Function {func_or_name} not found: {e}")
            return
    else:
        raise NotImplementedError("Only string names are supported for tracing.")


def show_trace():
    return json.dumps([x for x in traced_functions.keys()], indent=2)

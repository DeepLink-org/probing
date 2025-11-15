__all__ = [
    "query",
    "load_extension",
    "span",
    "event",
    "VERSION",
    "get_current_script_name",
    "should_enable_probing",
]

VERSION = "0.2.1"


def get_current_script_name():
    """Get the name of the current running script."""
    import sys
    import os
    try:
        script_path = sys.argv[0]
        return os.path.basename(script_path)
    except (IndexError, AttributeError):
        return "<unknown>"


def should_enable_probing():
    """
    Check if probing should be enabled based on PROBING environment variable.
    Uses the same logic as python/probing_hook.py.
    
    Returns:
        bool: True if probing should be enabled, False otherwise.
    """
    import os
    import sys
    
    # Get the PROBING environment variable
    # Check PROBING_ORIGINAL first (saved by probing_hook.py before deletion)
    # then fall back to PROBING
    probe_value = os.environ.get("PROBING_ORIGINAL") or os.environ.get("PROBING", "0")
    
    # If set to "0", disabled
    if probe_value == "0":
        return False
    
    # Handle init: prefix (extract the probe setting part)
    if probe_value.startswith("init:"):
        parts = probe_value.split("+", 1)
        probe_value = parts[1] if len(parts) > 1 else "0"
        # Note: init script execution is handled by probing_hook.py, not here
    
    # Handle "1" or "followed" - enable in current process
    if probe_value.lower() in ["1", "followed"]:
        return True
    
    # Handle "2" or "nested" - enable in current and child processes
    if probe_value.lower() in ["2", "nested"]:
        return True
    
    # Handle regex: pattern
    if probe_value.lower().startswith("regex:"):
        pattern = probe_value.split(":", 1)[1]
        try:
            import re
            current_script = get_current_script_name()
            return re.search(pattern, current_script) is not None
        except Exception:
            # If regex is invalid, don't enable
            return False
    
    # Handle script name matching
    current_script = get_current_script_name()
    if probe_value == current_script:
        return True
    
    # Default: don't enable if value doesn't match any pattern
    return False


def initialize_probing():
    """
    Initialize probing by loading the dynamic library.
    Only loads the library if PROBING environment variable indicates it should be enabled.

    Returns:
        ctypes.CDLL or None: The loaded library handle, or None if not enabled/not found.
    """
    import ctypes
    import pathlib
    import sys
    
    # Check if probing should be enabled based on environment variable
    if not should_enable_probing():
        return None

    # Determine library name based on OS
    if sys.platform == "darwin":
        lib_name = "libprobing.dylib"
    else:
        lib_name = "libprobing.so"

    # Search paths for the library
    current_file = pathlib.Path(__file__).resolve()

    paths = [
        pathlib.Path(sys.executable).parent / lib_name,
        current_file.parent / lib_name,
        pathlib.Path.cwd() / lib_name,
        pathlib.Path.cwd() / "target" / "debug" / lib_name,
        pathlib.Path.cwd() / "target" / "release" / lib_name,
    ]


    # Try loading the library from each path
    for path in paths:
        if path.exists():
            try:
                return ctypes.CDLL(str(path))
            except Exception:
                continue  # Try the next path if loading fails

    return None


handle = initialize_probing()
_library_loaded = handle is not None

if handle is None:
    import sys
    import functools
    import types
    
    # Define an empty module to indicate dummy mode
    # Use ModuleType to create a proper module object
    probing_module = types.ModuleType("probing")
    sys.modules["probing"] = probing_module
    
    # Re-add necessary attributes and functions to the empty module
    probing_module.__all__ = __all__
    probing_module.VERSION = VERSION
    probing_module._library_loaded = False  # Library not loaded in dummy mode
    probing_module.get_current_script_name = get_current_script_name
    probing_module.should_enable_probing = should_enable_probing

    def query(*args, **kwargs):
        raise ImportError("Probing library is not loaded.")
    
    def load_extension(*args, **kwargs):
        raise ImportError("Probing library is not loaded.")
    
    def span(*args, **kwargs):
        """Dummy span implementation that supports both context manager and decorator usage."""
        # Handle @span (without arguments) - no args and no kwargs
        if len(args) == 0 and not kwargs:
            def decorator(func):
                @functools.wraps(func)
                def wrapper(*wargs, **wkwargs):
                    return func(*wargs, **wkwargs)
                return wrapper
            return decorator
        
        # Handle @span(func) - first arg is a callable
        if len(args) == 1 and callable(args[0]):
            func = args[0]
            @functools.wraps(func)
            def wrapper(*wargs, **wkwargs):
                return func(*wargs, **wkwargs)
            return wrapper
        
        # Handle @span("name") or with span("name")
        if len(args) == 1 and isinstance(args[0], str):
            name = args[0]
            
            class DummySpanWrapper:
                def __init__(self, name: str, **attrs):
                    self.name = name
                    self.attrs = attrs
                
                def __call__(self, func):
                    """Enable decorator form when a name was provided."""
                    @functools.wraps(func)
                    def wrapper(*wargs, **wkwargs):
                        return func(*wargs, **wkwargs)
                    return wrapper
                
                def __enter__(self):
                    return self
                
                def __exit__(self, *exc):
                    return False
            
            return DummySpanWrapper(name, **kwargs)
        
        # Default: use as context manager with first arg as name
        if len(args) > 0:
            name = args[0]
            if not isinstance(name, str):
                raise TypeError("span() requires a string name as the first argument")
            
            class DummySpan:
                def __init__(self, name: str, **attrs):
                    self.name = name
                    self.attrs = attrs
                
                def __enter__(self):
                    return self
                
                def __exit__(self, *exc):
                    return False
            
            return DummySpan(name, **kwargs)
        
        raise TypeError("span() requires at least one argument")

    def event(*args, **kwargs):
        return
    
    # Add functions to the module
    probing_module.query = query
    probing_module.load_extension = load_extension
    probing_module.span = span
    probing_module.event = event
    
    # Also update the current module's namespace for direct access
    # This allows the functions to be accessible in the current module scope
    globals()["query"] = query
    globals()["load_extension"] = load_extension
    globals()["span"] = span
    globals()["event"] = event

else:
    import probing.hooks.import_hook
    import probing.inspect

    from probing.core.engine import query
    from probing.core.engine import load_extension

    from probing.tracing import span
    from probing.tracing import event
    
    # Set _library_loaded to True when library is successfully loaded
    _library_loaded = True
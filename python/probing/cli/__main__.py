import sys
import ctypes
import pathlib


def _force_load_library():
    """Force load the probing library for CLI usage."""
    # Determine library name based on OS
    if sys.platform == "darwin":
        lib_name = "libprobing.dylib"
    else:
        lib_name = "libprobing.so"
    
    # Search paths for the library
    current_file = pathlib.Path(__file__).resolve()
    # Go up from python/probing/cli/__main__.py to find library
    # python/probing/cli -> python/probing -> python -> root
    root_dir = current_file.parent.parent.parent.parent
    
    paths = [
        pathlib.Path(sys.executable).parent / lib_name,
        root_dir / lib_name,
        root_dir / "target" / "debug" / lib_name,
        root_dir / "target" / "release" / lib_name,
        pathlib.Path.cwd() / lib_name,
        pathlib.Path.cwd() / "target" / "debug" / lib_name,
        pathlib.Path.cwd() / "target" / "release" / lib_name,
    ]
    
    # Try loading the library from each path
    for path in paths:
        if path.exists():
            try:
                ctypes.CDLL(str(path))
                # Library loaded - the #[ctor] will call create_probing_module
                # which adds cli_main to the probing module
                return True
            except Exception as e:
                continue
    
    return False


def main():
    """Entry point for the probing CLI command."""
    # Force load the library for CLI usage (bypass environment variable check)
    if not _force_load_library():
        raise ImportError(
            "Cannot load probing library. Please ensure libprobing is available. "
            "The CLI tool requires the probing library to be installed."
        )
    
    # Import probing module - cli_main should now be available
    import probing
    
    # cli_main is added dynamically by Rust's create_probing_module
    cli_main = getattr(probing, "cli_main", None)
    
    if cli_main is None:
        raise ImportError(
            "cli_main is not available in probing module. "
            "The library may not have initialized correctly."
        )
    
    cli_main(["probing"] + sys.argv[1:])


if __name__ == "__main__":
    main()

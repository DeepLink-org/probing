import sys


def main():
    """Entry point for the probing CLI command."""
    # Import probing module - this will load _core via maturin
    # cli_main is exported from probing.__init__.py which imports it from _core
    import probing
    
    # cli_main is available from probing module (exported from __init__.py)
    if not hasattr(probing, "cli_main"):
        raise ImportError(
            "cli_main is not available in probing module. "
            "Please ensure the probing package is properly installed."
        )
    
    probing.cli_main(["probing"] + sys.argv[1:])


if __name__ == "__main__":
    main()

import os
import sys

# Set CLI mode before any probing imports to skip probe initialization
os.environ["PROBING_CLI_MODE"] = "1"
# Rust CLI subcommands (e.g. skill install) delegate to ``python -m probing.skills``.
os.environ.setdefault("PROBING_PYTHON", sys.executable)


def main():
    """Entry point for the probing CLI command."""
    import probing

    probing.cli_main(["probing"] + sys.argv[1:])


if __name__ == "__main__":
    main()

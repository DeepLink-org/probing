import os
import sys

# Set CLI mode before any probing imports to skip probe initialization
os.environ["PROBING_CLI_MODE"] = "1"


def main():
    """Entry point for the probing CLI command."""
    import probing

    probing.cli_main(["probing"] + sys.argv[1:])


if __name__ == "__main__":
    main()

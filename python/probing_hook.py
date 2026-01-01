"""
Probing Hook - Conditionally activates the probing library based on environment variables.

This module intercepts process startup and conditionally imports the probing library
based on the PROBING environment variable:
- '0': Disabled (default)
- '1' or 'followed': Enable only in current process
- '2' or 'nested': Enable in current and all child processes
- 'regex:PATTERN': Enable if current script name matches the regex pattern
- 'SCRIPTNAME': Enable if current script name matches exactly
- 'init:SCRIPT_PATH[+PROBE_SETTING]': Execute SCRIPT_PATH for initialization, then apply PROBE_SETTING.
                                      PROBE_SETTING can be any of the above values (e.g., 'init:./my_setup.py+1').
                                      If PROBE_SETTING is omitted, it defaults to '0' (disabled after init).
"""

import os
import sys


def get_current_script_name():
    """Get the name of the current running script."""
    try:
        script_path = sys.argv[0]
        return os.path.basename(script_path)
    except (IndexError, AttributeError):
        return "<unknown>"


# Get the PROBING environment variable
probe_value = os.environ.get("PROBING", "0")
current_script = get_current_script_name()
script_init = None

# Save original PROBING value to PROBING_ORIGINAL before any modifications
# This allows probing/__init__.py to check the original value even if PROBING is deleted
if "PROBING" in os.environ:
    os.environ["PROBING_ORIGINAL"] = os.environ["PROBING"]

if probe_value.startswith("init:"):
    parts = probe_value.split("+", 1)
    script_init = parts[0][5:]
    probe_value = parts[1] if len(parts) > 1 else "0"


def execute_init_script():
    if script_init is not None:
        with open(script_init) as f:
            exec(f.read(), globals())


def init_probing():
    try:
        # Remove the variable by default - we'll set it back if needed
        # PROBING_ORIGINAL is preserved for probing/__init__.py to check
        if "PROBING" in os.environ:
            del os.environ["PROBING"]

        if probe_value.lower() in ["1", "followed"]:
            print(
                "Activating probing in 'followed' mode (current process only)",
                file=sys.stderr,
            )
            import probing

            execute_init_script()
            # Environment variable is intentionally not preserved

        elif probe_value.lower() in ["2", "nested"]:
            print(
                "Activating probing in 'nested' mode (all child processes)",
                file=sys.stderr,
            )
            import probing

            # Preserve for child processes
            os.environ["PROBING"] = probe_value
            execute_init_script()

        elif probe_value.lower().startswith("regex:"):
            pattern = probe_value.split(":", 1)[1]
            try:
                import re

                if re.search(pattern, current_script) is not None:
                    print(
                        f"Activating probing for script matching '{pattern}'",
                        file=sys.stderr,
                    )
                    import probing

                    execute_init_script()
                # Always preserve valid regex patterns for child processes
                os.environ["PROBING"] = probe_value
            except Exception as e:
                print(f"Error in regex pattern '{pattern}': {e}", file=sys.stderr)
                # Don't preserve invalid regex patterns

        elif probe_value != "0":
            # Script name comparison
            if probe_value == current_script:
                print(
                    f"Activating probing for '{current_script}' (current process only)",
                    file=sys.stderr,
                )
                import probing

                execute_init_script()
            # Always preserve the script name filter for child processes
            os.environ["PROBING"] = probe_value

    except ImportError as e:
        print(f"Error loading probing library: {e}", file=sys.stderr)
    except Exception as e:
        print(f"Unexpected error in probing hook: {e}", file=sys.stderr)
        # In case of unexpected errors, don't enable probing


def is_probing_cli():
    """Check if the current process is the probing CLI itself."""
    if current_script == "probing":
        os.environ["PROBING_CLI_MODE"] = "1"
        return True
    # Check for 'python -m probing.cli'
    try:
        import __main__

        if hasattr(__main__, "__file__") and __main__.__file__:
            if "probing" in __main__.__file__ and "cli" in __main__.__file__:
                os.environ["PROBING_CLI_MODE"] = "1"
                return True
    except Exception:
        pass
    return False


try:
    import re

    # Skip initialization for:
    # 1. torchrun (launcher script)
    # 2. probing CLI itself (should not inject probes into itself)
    if re.search("torchrun", current_script) is None and not is_probing_cli():
        init_probing()
except Exception as e:
    print(f"Error in probing hook: {e}", file=sys.stderr)
    # In case of unexpected errors, don't enable probing

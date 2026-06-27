"""Process startup hook: conditionally import probing based on ``PROBING``.

Installed wheels run this via ``probing.pth`` → ``import probing_hook``.
Develop mode adds ``probing_hook.pth`` with the same import (see ``python/probing/dev_pth.py``).
"""

from __future__ import annotations

import os
import sys

from probing._entrypoint import (
    current_script_name,
    is_lightweight_module,
    is_probing_cli,
    should_activate_probing,
)

_RAN = False


def run_site_hook() -> None:
    """Entry point for ``probing_hook`` / ``probing_hook.pth``."""
    global _RAN
    if _RAN:
        return
    _RAN = True

    try:
        import re

        script = current_script_name()
        if (
            re.search("torchrun", script) is not None
            or is_probing_cli()
            or is_lightweight_module()
        ):
            return
        _init_probing()
    except Exception as exc:
        print(f"Error in probing site hook: {exc}", file=sys.stderr)


def _init_probing() -> None:
    probe_value = os.environ.get("PROBING", "0")
    script_init: str | None = None

    if "PROBING" in os.environ:
        os.environ["PROBING_ORIGINAL"] = os.environ["PROBING"]

    if probe_value.startswith("init:"):
        parts = probe_value.split("+", 1)
        script_init = parts[0][5:]
        probe_value = parts[1] if len(parts) > 1 else "0"

    current_script = current_script_name()

    try:
        if "PROBING" in os.environ:
            del os.environ["PROBING"]

        if probe_value.lower() in ("1", "followed"):
            print(
                "Activating probing in 'followed' mode (current process only)",
                file=sys.stderr,
            )
            import probing  # noqa: F401

            _execute_init_script(script_init)
            _install_crash_handler()

        elif probe_value.lower() in ("2", "nested"):
            print(
                "Activating probing in 'nested' mode (all child processes)",
                file=sys.stderr,
            )
            import probing  # noqa: F401

            os.environ["PROBING"] = probe_value
            _execute_init_script(script_init)
            _install_crash_handler()

        elif probe_value.lower().startswith("regex:"):
            pattern = probe_value.split(":", 1)[1]
            try:
                if should_activate_probing():
                    print(
                        f"Activating probing for script matching '{pattern}'",
                        file=sys.stderr,
                    )
                    import probing  # noqa: F401

                    _execute_init_script(script_init)
                    _install_crash_handler()
                os.environ["PROBING"] = probe_value
            except Exception as exc:
                print(f"Error in regex pattern '{pattern}': {exc}", file=sys.stderr)

        elif probe_value != "0":
            if probe_value == current_script:
                print(
                    f"Activating probing for '{current_script}' (current process only)",
                    file=sys.stderr,
                )
                import probing  # noqa: F401

                _execute_init_script(script_init)
                _install_crash_handler()
            os.environ["PROBING"] = probe_value

    except ImportError as exc:
        print(f"Error loading probing library: {exc}", file=sys.stderr)
    except Exception as exc:
        print(f"Unexpected error in probing site hook: {exc}", file=sys.stderr)


def _install_crash_handler() -> None:
    try:
        from probing.crash import install

        install()
    except Exception as exc:
        print(f"probing crash handler install skipped: {exc}", file=sys.stderr)


def _execute_init_script(script_init: str | None) -> None:
    if script_init is None:
        return
    with open(script_init, encoding="utf-8") as handle:
        exec(handle.read(), globals())  # noqa: S102

"""IPython magic commands for function tracing and debugging.

This module provides magic commands to trace Python function execution,
watch variables, monitor function calls, and enable remote debugging.
"""

from IPython.core.magic import Magics, magics_class, line_magic, cell_magic
from IPython.core.magic_arguments import argument, magic_arguments, parse_argstring
from probing.repl import register_magic
import json
import __main__
from typing import Any, Dict


@register_magic("debug")
@magics_class
class DebugMagic(Magics):
    """Magic commands for tracing function execution and remote debugging."""

    @line_magic
    def debug(self, line: str):
        """Unified debug command with subcommands.

        Usage:
            %debug remote [host=127.0.0.1] [port=9999] [try_install=True]    # Enable remote debugging
            %debug status                                                    # Show debugger status
            %debug help                                                      # Show help message
        """
        if not line or not line.strip():
            self._show_help()
            return

        parts = line.strip().split()
        subcommand = parts[0].lower()

        if subcommand == "remote":
            self._cmd_remote(" ".join(parts[1:]))
        elif subcommand == "status":
            self._cmd_status()
        elif subcommand in ["help", "--help", "-h"]:
            self._show_help()
        else:
            print(f"Unknown subcommand: {subcommand}")
            self._show_help()

    def _show_help(self) -> None:
        """Show help message for debug command."""
        help_text = """
Debug Magic Commands
====================

Usage:
  %debug remote [host=127.0.0.1] [port=9999] [try_install=True]    Enable remote debugging
  %debug status                                                    Show debugger status
  %debug help                                                      Show this help message

Examples:
  %debug remote                                    # Start remote debugger on default host:port
  %debug remote host=0.0.0.0 port=9999           # Start on specific host and port
  %debug remote try_install=False                 # Start without trying to install debugpy
  %debug status                                    # Check if debugpy is installed

Trace Commands:
  %trace start <function> --watch <vars> --depth <n>    Start tracing a function
  %trace stop <function>                                 Stop tracing a function
  %trace show                                            Show currently traced functions
  %trace list [<prefix>] [--limit <n>]                  List traceable functions
  %trace help                                            Show trace help message

Cell Magic (separate):
  %%probe --watch <vars> --depth <n>                    Execute code with probing enabled
        """
        print(help_text)

    def _cmd_remote(self, args_str: str) -> None:
        """Handle remote debug subcommand."""
        try:
            args = self._parse_args(args_str)
            host = args.get("host", "127.0.0.1")
            port = int(args.get("port", 9999))
            try_install_str = args.get("try_install", "True")
            try_install = try_install_str.lower() in ("true", "1", "t")
        except (ValueError, KeyError) as e:
            print(f"✗ Error parsing arguments: {e}")
            print(
                "Usage: %debug remote [host=127.0.0.1] [port=9999] [try_install=True]"
            )
            return

        if not self.detect_debugger() and try_install:
            print("debugpy is not installed. Attempting to install...")
            self.install_debugger()

        if self.detect_debugger():
            self.enable_debugger(host, port)
        else:
            print("✗ debugpy is not installed and installation was skipped.")
            print("  Please install it manually with: pip install debugpy")

    def _cmd_status(self) -> None:
        """Handle status subcommand."""
        status = self.status()
        installed = status.get("debugger_installed", False)
        address = status.get("debugger_address", None)

        print("\n=== Debugger Status ===\n")
        print(f"debugpy installed: {'✓ Yes' if installed else '✗ No'}")
        if address:
            print(f"Debugger address: {address}")
        else:
            print("Debugger address: Not started")
        print()

    def _parse_args(self, args_str: str) -> Dict[str, str]:
        """Parse key=value arguments from string."""
        args = {}
        if not args_str or not args_str.strip():
            return args

        for item in args_str.split():
            if "=" not in item:
                continue
            try:
                key, value = item.split("=", 1)
                args[key.strip()] = value.strip()
            except ValueError:
                continue

        return args

    # Trace command - unified entry point
    @line_magic
    def trace(self, line: str):
        """Unified trace command with subcommands.

        Usage:
            %trace watch                                       # Show currently watched functions
            %trace watch <function> [vars...] [--depth <n>]   # Watch a function
            %trace unwatch <function>                          # Stop watching a function
            %trace list [<prefix>] [--limit <n>]               # List traceable functions
            %trace help                                         # Show help message
        """
        if not line or not line.strip():
            self._show_trace_help()
            return

        parts = line.strip().split()
        subcommand = parts[0].lower()

        if subcommand == "watch":
            self._cmd_trace_watch(" ".join(parts[1:]))
        elif subcommand == "log":
            self._cmd_trace_log(" ".join(parts[1:]))
        elif subcommand == "unwatch":
            self._cmd_trace_unwatch(" ".join(parts[1:]))
        elif subcommand in ["list", "ls"]:
            self._cmd_trace_list(" ".join(parts[1:]))
        elif subcommand in ["help", "--help", "-h"]:
            self._show_trace_help()
        else:
            print(f"Unknown subcommand: {subcommand}")
            self._show_trace_help()

    def _show_trace_help(self) -> None:
        """Show help message for trace command."""
        help_text = """
Trace Magic Commands
====================

Usage:
  %trace watch                                       Show currently watched functions
  %trace watch <function> [var1 var2 ...] [--depth <n>]    Watch a function (prints to terminal)
  %trace log <function> [var1 var2 ...] [--depth <n>]      Log a function (only saves to table)
  %trace unwatch <function>                          Stop watching a function
  %trace list [<prefix>] [--limit <n>]              List traceable functions
  %trace ls [<prefix>] [-n <n>]                     Alias for list
  %trace help                                        Show this help message

Examples:
  # Show what's being watched
  %trace watch
  
  # Watch a function with specific variables (prints to terminal)
  %trace watch __main__.train loss accuracy lr
  %trace watch torch.nn.Linear.forward input output
  
  # Log a function (only saves to table, no terminal output)
  %trace log __main__.train loss accuracy lr
  %trace log mymodule.process data --depth 3
  
  # Watch a function with custom depth
  %trace watch mymodule.process data --depth 3
  
  # Stop watching
  %trace unwatch __main__.train
  
  # List traceable functions
  %trace list torch.nn
  %trace ls torch.optim --limit 20

Cell Magic (separate):
  %%probe --watch <vars> --depth <n>                 Execute code with probing enabled
        """
        print(help_text)

    def _cmd_trace_watch(self, args_str: str) -> None:
        """Handle trace watch subcommand - function trace or show mode."""
        from probing.inspect.trace import trace as trace_func
        from probing.inspect.trace import show_trace
        import json

        # Parse arguments
        parts = args_str.strip().split()
        
        # No arguments = show mode
        if not parts:
            result = show_trace()
            traced = json.loads(result)
            if not traced:
                print("No functions are currently being watched.")
                return
            output = ["Currently watched functions:"]
            for i, func in enumerate(traced, 1):
                output.append(f"  {i}. {func}")
            print("\n".join(output))
            return
        
        # Parse function name and arguments
        function = None
        watch_vars = []
        depth = 2
        
        i = 0
        # First arg that doesn't start with -- is the function name
        if parts and not parts[0].startswith("-"):
            function = parts[0]
            i = 1
        
        # Parse remaining args: --depth or variable names
        while i < len(parts):
            if parts[i] in ["--depth", "-d"]:
                i += 1
                if i < len(parts):
                    try:
                        depth = int(parts[i])
                        i += 1
                    except ValueError:
                        print(f"✗ Error: Invalid depth value: {parts[i]}")
                        return
            elif not parts[i].startswith("-"):
                # It's a variable name to watch
                watch_vars.append(parts[i])
                i += 1
            else:
                i += 1

        # Execute trace command (watch_vars go to watch list, not silent_watch)
        if function:
            try:
                trace_func(function, watch=watch_vars, silent_watch=[], depth=depth)
                watch_info = f" (vars: {', '.join(watch_vars)})" if watch_vars else ""
                print(f"✓ Started watching: {function}{watch_info}")
            except Exception as e:
                print(f"✗ Failed to watch {function}: {e}")
        else:
            print("✗ Error: Function name is required")
            print("Usage: %trace watch <function> [var1 var2 ...] [--depth <n>]")

    def _cmd_trace_log(self, args_str: str) -> None:
        """Handle trace log subcommand - similar to watch but silent (only saves to table)."""
        from probing.inspect.trace import trace as trace_func
        from probing.inspect.trace import show_trace
        import json

        # Parse arguments
        parts = args_str.strip().split()
        
        # No arguments = show mode (same as watch)
        if not parts:
            result = show_trace()
            traced = json.loads(result)
            if not traced:
                print("No functions are currently being logged.")
                return
            output = ["Currently logged functions:"]
            for i, func in enumerate(traced, 1):
                output.append(f"  {i}. {func}")
            print("\n".join(output))
            return
        
        # Parse function name and arguments
        function = None
        watch_vars = []
        depth = 2
        
        i = 0
        # First arg that doesn't start with -- is the function name
        if parts and not parts[0].startswith("-"):
            function = parts[0]
            i = 1
        
        # Parse remaining args: --depth or variable names
        while i < len(parts):
            if parts[i] in ["--depth", "-d"]:
                i += 1
                if i < len(parts):
                    try:
                        depth = int(parts[i])
                        i += 1
                    except ValueError:
                        print(f"✗ Error: Invalid depth value: {parts[i]}")
                        return
            elif not parts[i].startswith("-"):
                # It's a variable name to watch
                watch_vars.append(parts[i])
                i += 1
            else:
                i += 1

        # Execute trace command (watch_vars go to silent_watch list, not watch)
        if function:
            try:
                trace_func(function, watch=[], silent_watch=watch_vars, depth=depth)
                watch_info = f" (vars: {', '.join(watch_vars)})" if watch_vars else ""
                print(f"✓ Started logging: {function}{watch_info}")
            except Exception as e:
                print(f"✗ Failed to log {function}: {e}")
        else:
            print("✗ Error: Function name is required")
            print("Usage: %trace log <function> [var1 var2 ...] [--depth <n>]")

    def _cmd_trace_unwatch(self, args_str: str) -> None:
        """Handle trace unwatch subcommand."""
        from probing.inspect.trace import untrace as untrace_func

        function = args_str.strip()
        if not function:
            print("✗ Error: Function name is required")
            print("Usage: %trace unwatch <function>")
            return

        try:
            untrace_func(function)
            print(f"✓ Stopped watching: {function}")
        except Exception as e:
            print(f"✗ Failed to unwatch {function}: {e}")

    def _cmd_trace_show(self) -> None:
        """Handle trace show subcommand."""
        from probing.inspect.trace import show_trace

        result = show_trace()
        traced = json.loads(result)

        if not traced:
            print("No functions are currently being watched.")
            return

        output = ["Currently watched functions:"]
        for i, func in enumerate(traced, 1):
            output.append(f"  {i}. {func}")
        print("\n".join(output))

    def _cmd_trace_list(self, args_str: str) -> None:
        """Handle trace list subcommand."""
        from probing.inspect.trace import list_traceable as list_traceable_func

        # Parse arguments - support both --prefix flag and direct prefix (shell-like)
        parts = args_str.strip().split()
        prefix = None
        max_display = 50  # Default limit

        i = 0
        while i < len(parts):
            if parts[i] in ["--prefix", "-p"]:
                i += 1
                if i < len(parts):
                    prefix = parts[i]
                    i += 1
            elif parts[i] in ["--limit", "-n"]:
                i += 1
                if i < len(parts):
                    try:
                        max_display = int(parts[i])
                    except ValueError:
                        print(
                            f"Error: Invalid limit value '{parts[i]}'. Using default ({max_display})."
                        )
                    i += 1
            elif not parts[i].startswith("--") and not parts[i].startswith("-"):
                # Treat as direct prefix (shell-like behavior)
                if prefix is None:
                    prefix = parts[i]
                i += 1
            else:
                i += 1

        # Always returns variables by default
        result = list_traceable_func(prefix=prefix)
        items = json.loads(result)

        if not items:
            prefix_msg = f" matching '{prefix}'" if prefix else ""
            print(f"No traceable items found{prefix_msg}.")
            return

        # Display results with limit
        output = [f"Found {len(items)} traceable items"]
        if prefix:
            output[0] += f" matching '{prefix}'"
        output.append("")

        for i, item in enumerate(items[:max_display], 1):
            if isinstance(item, dict):
                # New format with variables
                item_type = item.get('type', '?')
                name = item.get('name', '?')
                variables = item.get('variables', [])
                
                # Format: [TYPE] name
                line = f"  {i}. [{item_type}] {name}"
                
                # Add variables if available
                if variables:
                    vars_str = ", ".join(variables[:5])  # Show first 5 variables
                    if len(variables) > 5:
                        vars_str += f" ... (+{len(variables) - 5} more)"
                    line += f" (vars: {vars_str})"
                
                output.append(line)
            else:
                # Old format (string)
                output.append(f"  {i}. {item}")

        if len(items) > max_display:
            output.append(f"\n  ... and {len(items) - max_display} more")
            output.append(
                f"\nTip: Use --limit/-n to show more results, or wildcards (*, ?) to narrow down"
            )

        print("\n".join(output))

    @cell_magic
    @magic_arguments()
    @argument("--watch", "-w", nargs="+", default=[], help="Variables to watch")
    @argument("--depth", "-d", type=int, default=1, help="Tracing depth")
    def probe(self, line: str, cell: str):
        """Execute code with probing enabled.

        Usage:
            %%probe --watch x y --depth 2
            def my_function(x):
                y = x * 2
                return y

            result = my_function(5)
        """
        from probing.inspect.trace import probe as probe_decorator

        args = parse_argstring(self.probe, line)

        # Execute the cell code in the user's namespace
        # with probing enabled for all functions
        exec_code = f"""
from probing.inspect.trace import probe as _probe_decorator

# Wrap execution in probe context
_probing_tracer = _probe_decorator(watch={args.watch!r}, depth={args.depth})
with _probing_tracer:
{chr(10).join('    ' + line for line in cell.split(chr(10)))}
"""

        self.shell.run_cell(exec_code)

    # Remote debugging utility methods
    @staticmethod
    def status() -> Dict[str, Any]:
        """Get debugger status."""
        if not hasattr(__main__, "__probing__"):
            __main__.__probing__ = {}
        if "debug" not in __main__.__probing__:
            __main__.__probing__["debug"] = {}
        __main__.__probing__["debug"][
            "debugger_installed"
        ] = DebugMagic.detect_debugger()

        return __main__.__probing__["debug"]

    @staticmethod
    def detect_debugger():
        """Check if debugpy is installed."""
        try:
            import debugpy

            return True
        except ImportError:
            return False

    @staticmethod
    def install_debugger():
        """Install debugpy package."""
        try:
            from pip import main as pipmain
        except ImportError:
            from pip._internal import main as pipmain
        pipmain(["install", "debugpy"])

    @staticmethod
    def enable_debugger(host: str = "127.0.0.1", port: int = 9999):
        """Enable remote debugger on specified host and port."""
        status = DebugMagic.status()
        try:
            import debugpy
        except Exception:
            print("debugpy is not installed, please install debugpy with pip:")
            print("\tpip install debugpy")
            return
        debugpy.listen((host, port))
        status["debugger_address"] = f"{host}:{port}"
        print(f"✓ Remote debugger started at {host}:{port}")

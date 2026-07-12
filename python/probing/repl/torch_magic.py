"""IPython magic commands for PyTorch profiling and inspection.

This module provides unified torch magic commands for:
- Profiling PyTorch modules
- Viewing top-level models
- Checking GPU memory usage
"""

from typing import Dict, Optional

from IPython.core.magic import Magics, line_magic, magics_class

import __main__
from probing.repl import register_magic

from probing.profiling.torch_profiler import ProfilerController, get_controller

# Optional import - torch may not be installed
try:
    import torch

    HAS_TORCH = True
except ImportError:
    HAS_TORCH = False
    torch = None


@register_magic("torch")
@magics_class
class TorchMagic(Magics):
    """Magic commands for PyTorch operations."""

    PROFILER_KEY = "global_profiler"

    def __init__(self, shell):
        super().__init__(shell)
        if not hasattr(__main__, "__probing__"):
            __main__.__probing__ = {}
        if self.PROFILER_KEY not in __main__.__probing__:
            __main__.__probing__[self.PROFILER_KEY] = None

    @line_magic
    def pytorch(self, line: str):
        """Unified PyTorch command with subcommands.

        Usage:
            %pytorch profile [steps=N]               # Start global profiler for N steps
            %pytorch summary                         # Show profiler summary
            %pytorch timeline                        # Export timeline in Chrome tracing format
            %pytorch memory                          # Show GPU memory info
            %pytorch help                            # Show help message
        """
        if not HAS_TORCH:
            print("PyTorch is not installed. Please install it with: pip install torch")
            return

        if not line or not line.strip():
            self._show_help()
            return

        parts = line.strip().split()
        subcommand = parts[0].lower()

        if subcommand == "profile":
            self._cmd_profile(" ".join(parts[1:]))
        elif subcommand == "summary":
            self._cmd_summary()
        elif subcommand == "timeline":
            self._cmd_timeline()
        elif subcommand == "memory":
            self._cmd_memory()
        elif subcommand in ["help", "--help", "-h"]:
            self._show_help()
        else:
            print(f"Unknown subcommand: {subcommand}")
            self._show_help()

    def _show_help(self) -> None:
        """Show help message for pytorch command."""
        help_text = """
PyTorch Magic Commands
======================

Usage:
  %pytorch profile [steps=N]               Start global profiler for N steps
  %pytorch summary                         Show profiler summary
  %pytorch timeline                        Export timeline in Chrome tracing format
  %pytorch memory                          Show GPU memory information
  %pytorch help                            Show this help message

Examples:
  %pytorch profile steps=5                 # Start profiler for 5 steps
  # Profiler will automatically record data after each optimizer.step()
  %pytorch summary                         # Show profiler summary
  %pytorch timeline                        # Export timeline data
  %pytorch memory                          # Show CUDA memory info
        """
        print(help_text)

    def _cmd_profile(self, args_str: str) -> None:
        """Handle profile subcommand - start global profiler."""
        try:
            args = self._parse_args(args_str)
            steps = int(args.get("steps", 1))
        except (ValueError, KeyError) as e:
            print(f"✗ Error parsing arguments: {e}")
            print("Usage: %pytorch profile [steps=1]")
            return

        controller = get_controller()
        if controller.is_running:
            print("✗ Global profiler is already running")
            print(
                "  The profiler can only be started once. Stop it first or wait for it to complete."
            )
            return

        try:
            controller.start(steps=steps, trigger="repl")
            __main__.__probing__[self.PROFILER_KEY] = controller
            __main__.profiler = controller
            print(f"✓ Global profiler started for {steps} step(s)")
            print(
                "  Profiler will automatically record data after each optimizer.step()"
            )
        except Exception as e:
            print(f"✗ Failed to start profiler: {e}")
            import traceback

            traceback.print_exc()

    def _cmd_summary(self) -> None:
        """Handle summary subcommand."""
        controller = __main__.__probing__.get(self.PROFILER_KEY) or get_controller()
        if not isinstance(controller, ProfilerController):
            print("No profiler found. Use '%pytorch profile' first.")
            return

        profiler = controller._profiler  # noqa: SLF001 — REPL summary
        if profiler is None:
            print("Profiler has no active session.")
            return

        try:
            events = profiler.events()
            event_list = list(events) if events else []
            if event_list:
                print(f"Profiler collected {len(event_list)} events")
                table = profiler.key_averages().table(
                    sort_by="self_cuda_time_total", row_limit=10
                )
                print(table)
            else:
                status = controller.status()
                print("Profiler has no events.")
                print(
                    f"  Step count: {status['steps_completed']}/{status['steps_target']}"
                )
        except Exception as e:
            print(f"Error generating summary: {e}")

    def _cmd_timeline(self) -> None:
        """Handle timeline subcommand - export Chrome tracing format."""
        controller = __main__.__probing__.get(self.PROFILER_KEY) or get_controller()
        if not isinstance(controller, ProfilerController):
            print("No profiler found. Use '%pytorch profile' first.")
            return

        timeline = controller.export_timeline()
        if timeline:
            print(timeline)
        else:
            print(
                "No timeline data available. Make sure the profiler has been executed."
            )

    def _cmd_memory(self) -> None:
        """Handle memory subcommand."""
        if not HAS_TORCH:
            print("PyTorch is not installed. Please install it with: pip install torch")
            return

        try:
            if not torch.cuda.is_available():
                print("CUDA is not available. No GPU memory information to display.")
                return

            print("\n=== GPU Memory Information ===\n")

            # Get device count
            device_count = torch.cuda.device_count()
            print(f"CUDA Devices: {device_count}\n")

            for device_id in range(device_count):
                torch.cuda.set_device(device_id)
                device_name = torch.cuda.get_device_name(device_id)
                print(f"Device {device_id}: {device_name}")

                # Memory allocated and reserved
                allocated = torch.cuda.memory_allocated(device_id)
                reserved = torch.cuda.memory_reserved(device_id)
                max_allocated = torch.cuda.max_memory_allocated(device_id)
                max_reserved = torch.cuda.max_memory_reserved(device_id)

                def format_bytes(bytes_val: int) -> str:
                    """Format bytes to human readable format."""
                    for unit in ["B", "KB", "MB", "GB", "TB"]:
                        if bytes_val < 1024.0:
                            return f"{bytes_val:.2f} {unit}"
                        bytes_val /= 1024.0
                    return f"{bytes_val:.2f} PB"

                print(f"  Allocated: {format_bytes(allocated)}")
                print(f"  Reserved:  {format_bytes(reserved)}")
                print(f"  Max Allocated: {format_bytes(max_allocated)}")
                print(f"  Max Reserved:  {format_bytes(max_reserved)}")
                print()

            # Memory summary
            try:
                summary = torch.cuda.memory_summary(device=None, abbreviated=True)
                print("=== Memory Summary ===")
                print(summary)
            except Exception:
                pass  # Some PyTorch versions may not support memory_summary

        except Exception as e:
            print(f"✗ Error getting memory information: {e}")

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

    def _start_global_profiler(
        self, steps: int = 1, trigger: str = "http"
    ) -> ProfilerController:
        """Start profiler (HTTP / legacy callers)."""
        if not HAS_TORCH:
            raise ImportError(
                "PyTorch is not installed. Please install it with: pip install torch"
            )
        controller = get_controller()
        if controller.is_running:
            raise RuntimeError("profiler already running")
        controller.start(steps=steps, trigger=trigger)
        __main__.__probing__[self.PROFILER_KEY] = controller
        __main__.profiler = controller
        return controller

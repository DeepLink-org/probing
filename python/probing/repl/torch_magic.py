"""IPython magic commands for PyTorch profiling and inspection.

This module provides unified torch magic commands for:
- Profiling PyTorch modules
- Viewing top-level models
- Checking GPU memory usage
"""

from IPython.core.magic import Magics, magics_class, line_magic
from probing.repl import register_magic
import gc
import __main__
from typing import Optional, List, Dict, Any

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

        # Check if profiler already exists (non-reentrant)
        if __main__.__probing__.get(self.PROFILER_KEY) is not None:
            print("✗ Global profiler is already running")
            print("  The profiler can only be started once. Stop it first or wait for it to complete.")
            return

        try:
            self._start_global_profiler(steps)
            print(f"✓ Global profiler started for {steps} step(s)")
            print("  Profiler will automatically record data after each optimizer.step()")
        except Exception as e:
            print(f"✗ Failed to start profiler: {e}")
            import traceback
            traceback.print_exc()

    def _cmd_summary(self) -> None:
        """Handle summary subcommand."""
        profiler = __main__.__probing__.get(self.PROFILER_KEY)
        if profiler is None:
            print("No profiler found. Use '%pytorch profile' first.")
            return

        profiler.summary()

    def _cmd_timeline(self) -> None:
        """Handle timeline subcommand - export Chrome tracing format."""
        profiler = __main__.__probing__.get(self.PROFILER_KEY)
        if profiler is None:
            print("No profiler found. Use '%pytorch profile' first.")
            return

        timeline = profiler.export_timeline()
        if timeline:
            # Print JSON to stdout (can be captured by API)
            print(timeline)
        else:
            print("No timeline data available. Make sure the profiler has been executed.")

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

    def _start_global_profiler(self, steps: int = 1):
        """Start a global profiler using context manager approach.
        
        Args:
            steps: Number of steps to profile.
        """
        if not HAS_TORCH:
            raise ImportError(
                "PyTorch is not installed. Please install it with: pip install torch"
            )

        class _GlobalProfiler:
            """Global profiler class using torch.profiler.profile context manager."""

            def __init__(self, steps: int) -> None:
                self._steps = steps
                self._step_count = 0
                self._profiler = None
                self._cached_timeline = None
                self._timeline_exported = False
                self._hook_handle = None  # Store hook handle for removal
                
                # Configure activities
                activities = [torch.profiler.ProfilerActivity.CPU]
                if torch.cuda.is_available():
                    activities.append(torch.profiler.ProfilerActivity.CUDA)
                
                self._profiler = torch.profiler.profile(
                    record_shapes=True,
                    with_stack=True,
                    with_flops=True,
                    activities=activities,
                    on_trace_ready=None
                )
                
                # Register optimizer step post hook to automatically drive profiler
                from torch.optim.optimizer import register_optimizer_step_post_hook
                
                profiler_instance = self
                
                def profiler_step_hook(optimizer, *args, **kwargs):
                    """Hook function that starts profiler and calls step() after optimizer.step()."""
                    # Start profiler on first call (in training thread)
                    if profiler_instance._step_count == 0:
                        profiler_instance._profiler.__enter__()
                        print(f"==== Profiler started (profiling {profiler_instance._steps} steps) ====")
                    
                    # Record step if profiler is active and hasn't reached limit
                    if profiler_instance._profiler is not None and profiler_instance._step_count < profiler_instance._steps:
                        try:
                            profiler_instance._profiler.step()
                            profiler_instance._step_count += 1
                            
                            # Stop profiler when step limit is reached
                            if profiler_instance._step_count >= profiler_instance._steps:
                                profiler_instance._profiler.__exit__(None, None, None)
                                print(f"==== Profiler stopped (completed {profiler_instance._steps} steps) ====")
                                # Remove hook
                                if profiler_instance._hook_handle is not None:
                                    try:
                                        from torch.optim.optimizer import remove_optimizer_step_post_hook
                                        remove_optimizer_step_post_hook(profiler_instance._hook_handle)
                                        profiler_instance._hook_handle = None
                                    except Exception:
                                        pass
                        except RuntimeError as e:
                            # Profiler already stopped - ignore
                            error_msg = str(e).lower()
                            if "can't disable" in error_msg or "not running" in error_msg:
                                if profiler_instance._hook_handle is not None:
                                    try:
                                        from torch.optim.optimizer import remove_optimizer_step_post_hook
                                        remove_optimizer_step_post_hook(profiler_instance._hook_handle)
                                        profiler_instance._hook_handle = None
                                    except Exception:
                                        pass
                
                self._hook_handle = register_optimizer_step_post_hook(profiler_step_hook)

            def step(self):
                """Manually call step() after each iteration (usually not needed - hook handles it)."""
                if self._profiler is None or self._step_count >= self._steps:
                    return
                
                try:
                    self._profiler.step()
                    self._step_count += 1
                    
                    if self._step_count >= self._steps:
                        self._profiler.__exit__(None, None, None)
                except Exception:
                    pass

            def stop(self):
                """Manually stop the profiler."""
                if self._profiler is None:
                    return
                
                if self._hook_handle is not None:
                    try:
                        from torch.optim.optimizer import remove_optimizer_step_post_hook
                        remove_optimizer_step_post_hook(self._hook_handle)
                        self._hook_handle = None
                    except Exception:
                        pass
                
                try:
                    self._profiler.__exit__(None, None, None)
                    print(f"==== Profiler stopped manually (completed {self._step_count}/{self._steps} steps) ====")
                except Exception:
                    pass

            def summary(self):
                """Print profiler summary."""
                if self._profiler is None:
                    print("Profiler is not initialized")
                    return

                try:
                    events = self._profiler.events()
                    event_list = list(events) if events else []
                    event_count = len(event_list)
                    
                    if event_count > 0:
                        print(f"Profiler collected {event_count} events")
                        table = self._profiler.key_averages().table(
                            sort_by="cpu_time_total", row_limit=10
                        )
                        print(table)
                    else:
                        print("Profiler has no events.")
                        print(f"  Step count: {self._step_count}/{self._steps}")
                        if self._step_count < self._steps:
                            print("  Profiler may still be running. Wait for it to complete.")
                except RuntimeError as e:
                    error_msg = str(e).lower()
                    if "not enabled" in error_msg or "not started" in error_msg or "not running" in error_msg:
                        print("Profiler has been stopped.")
                        print(f"  Step count: {self._step_count}/{self._steps}")
                    else:
                        raise
                except Exception as e:
                    print(f"Error generating summary: {e}")

            def export_timeline(self) -> Optional[str]:
                """Export profiler timeline in Chrome tracing format.
                
                Returns:
                    JSON string of Chrome tracing format, or None if profiler is not ready.
                """
                if self._timeline_exported and self._cached_timeline is not None:
                    return self._cached_timeline
                
                if self._profiler is None:
                    return None
                
                try:
                    import tempfile
                    import os
                    import json
                    
                    tmp_fd, tmp_path = tempfile.mkstemp(suffix='.json', text=True)
                    os.close(tmp_fd)
                    
                    try:
                        self._profiler.export_chrome_trace(tmp_path)
                        
                        with open(tmp_path, 'r') as f:
                            trace_json = f.read()
                        
                        # Verify it's valid JSON
                        parsed = json.loads(trace_json)
                        trace_events = parsed.get("traceEvents", [])
                        
                        if not trace_events:
                            return None
                        
                        # Cache the timeline
                        self._cached_timeline = trace_json
                        self._timeline_exported = True
                        
                        return trace_json
                    finally:
                        try:
                            os.unlink(tmp_path)
                        except Exception:
                            pass
                except Exception as e:
                    return None

        # Create and store global profiler
        profiler = _GlobalProfiler(steps)
        __main__.__probing__[self.PROFILER_KEY] = profiler
        __main__.profiler = profiler
        
        return profiler

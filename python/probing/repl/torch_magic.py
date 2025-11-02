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

    PROFILER_KEY = "profiler"

    def __init__(self, shell):
        super().__init__(shell)
        if not hasattr(__main__, "__probing__"):
            __main__.__probing__ = {}
        if self.PROFILER_KEY not in __main__.__probing__:
            __main__.__probing__[self.PROFILER_KEY] = {}

    @line_magic
    def pytorch(self, line: str):
        """Unified PyTorch command with subcommands.

        Usage:
            %pytorch profile [steps=1] [mid=None]    # Enable profiler
            %pytorch summary                         # Show profiler summary
            %pytorch models                          # List top-level modules
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
        elif subcommand == "models":
            self._cmd_models()
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
  %pytorch profile [steps=1] [mid=None]    Enable profiler on modules
  %pytorch summary                         Show profiler summary
  %pytorch models                          List all top-level modules
  %pytorch memory                          Show GPU memory information
  %pytorch help                            Show this help message

Examples:
  %pytorch profile steps=5                 # Profile for 5 steps
  %pytorch profile mid=123456              # Profile specific module by ID
  %pytorch models                          # List all top-level modules
  %pytorch memory                          # Show CUDA memory info
        """
        print(help_text)

    def _cmd_profile(self, args_str: str) -> None:
        """Handle profile subcommand."""
        try:
            args = self._parse_args(args_str)
            steps = int(args.get("steps", 1))
            mid_str = args.get("mid", None)
            mid = int(mid_str) if mid_str and mid_str.lower() != "none" else None
        except (ValueError, KeyError) as e:
            print(f"✗ Error parsing arguments: {e}")
            print("Usage: %pytorch profile [steps=1] [mid=None]")
            return
        
        print(f"Profiling for {steps} step(s)")
        try:
            self._profile(steps, mid)
            print("✓ Profiler installed successfully")
        except Exception as e:
            print(f"✗ Failed to install profiler: {e}")

    def _cmd_summary(self) -> None:
        """Handle summary subcommand."""
        if self.PROFILER_KEY not in __main__.__probing__:
            print("No profiler instances found. Use '%pytorch profile' first.")
            return
        
        profilers = __main__.__probing__[self.PROFILER_KEY]
        if not profilers:
            print("No profiler instances found. Use '%pytorch profile' first.")
            return
        
        for module_id, profiler in profilers.items():
            print(f"\n--- Profiler for module ID {module_id} ---")
            profiler.summary()

    def _cmd_models(self) -> None:
        """Handle models subcommand."""
        try:
            modules = self.get_top_level_modules()
            if not modules:
                print("No top-level PyTorch modules found.")
                return
            
            print(f"\nFound {len(modules)} top-level module(s):\n")
            for i, module in enumerate(modules, 1):
                module_id = id(module)
                module_name = module.__class__.__name__
                print(f"  [{i}] ID: {module_id}")
                print(f"      Type: {module_name}")
                print(f"      Module: {module}")
                print()
            
            print(f"To profile a specific module, use: %pytorch profile mid=<module_id>")
        except Exception as e:
            print(f"✗ Error listing modules: {e}")

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
                    for unit in ['B', 'KB', 'MB', 'GB', 'TB']:
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

    @staticmethod
    def get_top_level_modules() -> List:
        """Get all top-level PyTorch modules (modules that are not children of other modules).
        
        Returns:
            List of top-level torch.nn.Module instances.
        """
        if not HAS_TORCH:
            raise ImportError("PyTorch is not installed. Please install it with: pip install torch")
        
        try:
            objs = gc.get_objects()
            objs = [obj for obj in objs if isinstance(obj, torch.nn.Module)]
            children = set()

            def walk(obj):
                """Recursively walk module children."""
                try:
                    if hasattr(obj, "children"):
                        for child in obj.children():
                            if child is not None:
                                children.add(id(child))
                                walk(child)
                except (AttributeError, RuntimeError):
                    # Skip modules with issues
                    pass

            for obj in objs:
                try:
                    walk(obj)
                except Exception:
                    continue
            
            return [obj for obj in objs if id(obj) not in children]
        except Exception as e:
            print(f"Warning: Error getting top-level modules: {e}")
            return []

    @staticmethod
    def install_profiler(module, steps: int = 1):
        """Install profiler on a PyTorch module.
        
        Args:
            module: The PyTorch module to profile.
            steps: Number of forward passes to profile.
        
        Returns:
            Profiler instance.
        """
        if not HAS_TORCH:
            raise ImportError("PyTorch is not installed. Please install it with: pip install torch")
        class _Profiler:
            """Internal profiler class."""
            
            def __init__(self, steps: int) -> None:
                self._steps = steps
                self._profiler = None
                self._count = 0
                self._hooks = []
                self._module = None
                self._status = False
                self._profiler_started = False  # Track if profiler was started by this instance

            def install(self, module):
                """Install profiler hooks on module."""
                if not HAS_TORCH:
                    raise ImportError("PyTorch is not installed. Please install it with: pip install torch")
                self._module = module
                # Create a new profiler instance for this module
                # Use record_shapes and with_stack to get more detailed info
                self._profiler = torch.profiler.profile(
                    record_shapes=True,
                    with_stack=True
                )
                print(f"Installing profiler to module: {module.__class__.__name__}")
                # Register both pre and post hooks
                # Pre hook: start profiler on first call, count calls
                # Post hook: record step after forward pass completes
                pre_hook = module.register_forward_pre_hook(self.pre_forward_hook)
                post_hook = module.register_forward_hook(self.post_forward_hook)
                self._hooks.append(pre_hook)
                self._hooks.append(post_hook)
                return self

            def pre_forward_hook(self, module, input):
                """Hook called before each forward pass."""
                # Only process if we haven't reached the step limit
                if self._count >= self._steps:
                    return None
                
                # Start profiling on first call
                if not self._status and not self._profiler_started:
                    try:
                        print(f"==== Start profiling module {module.__class__.__name__} ====")
                        self._profiler.start()
                        self._profiler_started = True
                        self._status = True
                    except RuntimeError as e:
                        if "already enabled" in str(e).lower():
                            # Another profiler instance is running, skip this one
                            print(f"Warning: Another profiler is running, skipping profiler for {module.__class__.__name__}")
                            self._status = True  # Mark as done to prevent further attempts
                            return None
                        else:
                            raise
                
                return None
            
            def post_forward_hook(self, module, input, output):
                """Hook called after each forward pass."""
                # Only process if we haven't reached the step limit
                if self._count >= self._steps:
                    return None
                
                # Count the forward pass (only after it completes)
                self._count += 1
                
                # Record the step after forward pass completes
                if self._profiler_started:
                    try:
                        # step() should be called after forward pass completes
                        self._profiler.step()
                    except RuntimeError as e:
                        # Profiler may have been stopped
                        if "not enabled" in str(e).lower() or "not started" in str(e).lower():
                            print(f"Warning: Profiler was stopped during forward pass")
                            self._profiler_started = False
                        else:
                            # Re-raise other errors
                            raise
                
                # If we've reached the step limit, stop profiling
                if self._count >= self._steps and self._profiler_started:
                    try:
                        print(f"==== Stop profiling module {module.__class__.__name__} after {self._steps} steps ====")
                        self._profiler.stop()
                        self._status = False
                        self._profiler_started = False
                    except RuntimeError as e:
                        # Profiler may have been stopped already
                        if "not enabled" in str(e).lower() or "not started" in str(e).lower():
                            print(f"Warning: Profiler for {module.__class__.__name__} was already stopped")
                        self._status = False
                        self._profiler_started = False
                
                return None

            def summary(self):
                """Print profiler summary."""
                if self._profiler is None:
                    print("Profiler is not initialized")
                    return
                
                try:
                    events = self._profiler.events()
                    if events:
                        table = self._profiler.key_averages().table(
                            sort_by="cpu_time_total", 
                            row_limit=10
                        )
                        print(table)
                    else:
                        print("Profiler has no events. Make sure the model has been executed.")
                except Exception as e:
                    print(f"Error generating summary: {e}")

            def remove(self):
                """Remove profiler hooks."""
                for hook in self._hooks:
                    try:
                        hook.remove()
                    except Exception:
                        pass
                self._hooks.clear()

        return _Profiler(steps).install(module)

    @staticmethod
    def _profile(steps: int = 1, mid: Optional[int] = None):
        """Install profiler on specified modules.
        
        Args:
            steps: Number of forward passes to profile.
            mid: Optional module ID to profile. If None, profiles all top-level modules.
        """
        if mid is not None:
            # Find module by ID
            tms = [m for m in gc.get_objects() if id(m) == mid]
            if not tms:
                raise ValueError(f"Module with ID {mid} not found. Use '%pytorch models' to list available modules.")
        else:
            # Get all top-level modules
            tms = TorchMagic.get_top_level_modules()
            if not tms:
                raise ValueError("No top-level modules found. Make sure you have PyTorch modules in memory.")
        
        for m in tms:
            # Check if profiler already exists
            module_id = id(m)
            if module_id in __main__.__probing__.get(TorchMagic.PROFILER_KEY, {}):
                print(f"Warning: Profiler already exists for module {module_id}. Reinstalling...")
                # Optionally remove old profiler hooks
                old_profiler = __main__.__probing__[TorchMagic.PROFILER_KEY][module_id]
                if hasattr(old_profiler, 'remove'):
                    old_profiler.remove()
            
            p = TorchMagic.install_profiler(m, steps)
            __main__.__probing__[TorchMagic.PROFILER_KEY][module_id] = p
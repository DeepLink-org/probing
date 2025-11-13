"""IPython magic commands for object inspection.

This module provides a unified %inspect command to inspect PyTorch objects,
memory usage, and runtime state.
"""

from IPython.core.magic import Magics, magics_class, line_magic
from IPython.core.magic_arguments import argument, magic_arguments, parse_argstring
from probing.repl import register_magic
import json


@register_magic("inspect")
@magics_class
class InspectMagic(Magics):
    """Magic commands for inspecting objects and runtime state."""

    @line_magic
    @magic_arguments()
    @argument(
        "subcommand",
        nargs="?",
        default="help",
        help="Subcommand: ls/list, gc, cuda, or help",
    )
    @argument(
        "target",
        nargs="?",
        default=None,
        help="Target: modules, tensors, optimizers, objects",
    )
    @argument("--limit", "-n", type=int, default=None, help="Limit number of results")
    @argument(
        "--device", "-d", type=str, default=None, help="Filter by device (for tensors)"
    )
    @argument(
        "--type", "-t", type=str, default=None, help="Filter by type (for objects)"
    )
    @argument(
        "--all", action="store_true", help="Show all modules including submodules (for modules target)"
    )
    def inspect(self, line: str):
        """Inspect PyTorch modules, tensors, optimizers, and memory.

        Usage: %inspect <subcommand> [target] [options]
        Use '%inspect help' for detailed usage information.
        """
        args = parse_argstring(self.inspect, line)

        subcommand = args.subcommand.lower()

        # Handle aliases
        if subcommand == "list":
            subcommand = "ls"

        if subcommand == "help" or (subcommand == "ls" and not args.target):
            self._show_help()
        elif subcommand == "ls":
            self._handle_list(args)
        elif subcommand == "gc":
            self._handle_gc()
        elif subcommand == "cuda":
            self._handle_cuda()
        else:
            print(
                f"Unknown subcommand: {args.subcommand}\n\nUse '%inspect help' for usage."
            )

    def _show_help(self):
        """Show help message."""
        print(
            """
🔍 Inspect Command Help
======================================================================

Usage: %inspect <subcommand> [target] [options]

Subcommands:
  ls, list              List objects in memory
  gc                    Run garbage collection
  cuda                  Show CUDA memory usage
  help                  Show this help message

Targets (for 'ls' subcommand):
  modules               PyTorch nn.Module instances
  tensors               PyTorch Tensor instances
  optimizers            PyTorch Optimizer instances
  objects               All object types (grouped by type)

Options:
  -n, --limit N         Limit number of results
  -d, --device DEV      Filter by device (tensors only)
  -t, --type TYPE       Filter by type (objects only)
  --all                 Show all modules including submodules (modules only, default: top-level only)

Examples:
  %inspect ls modules                  # Show top-level modules only
  %inspect ls modules --all            # Show all modules including submodules
  %inspect ls tensors -d cuda -n 10
  %inspect ls objects -t torch.Tensor
  %inspect gc
  %inspect cuda

======================================================================
"""
        )

    def _handle_list(self, args):
        """Handle list subcommand."""
        target = args.target.lower() if args.target else None

        if not target:
            print(
                "Error: Please specify a target (modules, tensors, optimizers, or objects)\n\nUse '%inspect help' for usage."
            )
            return

        if target == "modules":
            self._list_modules(args)
        elif target == "tensors":
            self._list_tensors(args)
        elif target == "optimizers":
            self._list_optimizers(args)
        elif target == "objects":
            self._list_objects(args)
        else:
            print(
                f"Unknown target: {target}\n\nValid targets: modules, tensors, optimizers, objects"
            )

    def _list_modules(self, args):
        """List PyTorch modules. Default shows only top-level modules."""
        try:
            import torch
            import gc

            if args.all:
                # Get all modules including submodules
                from probing.inspect import get_torch_modules
                module_items = get_torch_modules()

                # Filter out non-module items and extract actual objects
                modules = []
                for item in module_items:
                    if isinstance(item, dict) and "value" in item:
                        module = item["value"]
                        # Skip if it's actually a dict or not a real module
                        if isinstance(module, dict) or not isinstance(
                            module, torch.nn.Module
                        ):
                            continue
                        modules.append((module, item.get("id", id(module))))
                    elif isinstance(item, torch.nn.Module):
                        modules.append((item, id(item)))
            else:
                # Default: Get only top-level modules (not children of other modules)
                modules = self._get_top_level_modules()

            if args.limit:
                modules = modules[: args.limit]

            if not modules:
                module_type = "" if args.all else "top-level "
                print(f"No {module_type}PyTorch modules found in memory.")
                return

            module_type_label = "" if args.all else "top-level "
            output = [f"Found {len(modules)} {module_type_label}PyTorch module(s):"]
            for i, (module, module_id) in enumerate(modules, 1):
                module_type = type(module).__name__
                # Count parameters
                try:
                    num_params = sum(p.numel() for p in module.parameters())
                    output.append(
                        f"  {i}. {module_type} (id: {module_id}, params: {num_params:,})"
                    )
                except:
                    output.append(f"  {i}. {module_type} (id: {module_id})")

            print("\n".join(output))
        except ImportError:
            print("✗ PyTorch is not available")
        except Exception as e:
            print(f"✗ Failed to get torch modules: {e}")

    def _get_top_level_modules(self):
        """Get all top-level PyTorch modules (modules that are not children of other modules).

        Returns:
            List of tuples (module, module_id) for top-level torch.nn.Module instances.
        """
        import torch
        import gc

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

            top_level = [obj for obj in objs if id(obj) not in children]
            return [(module, id(module)) for module in top_level]
        except Exception as e:
            print(f"Warning: Error getting top-level modules: {e}")
            return []

    def _list_tensors(self, args):
        """List PyTorch tensors."""
        try:
            from probing.inspect import get_torch_tensors
            import torch

            tensor_items = get_torch_tensors()

            # Filter out non-tensor items and extract actual objects
            tensors = []
            for item in tensor_items:
                if isinstance(item, dict) and "value" in item:
                    tensor = item["value"]
                    # Skip if it's not actually a tensor
                    if not isinstance(tensor, torch.Tensor):
                        continue
                    tensors.append(tensor)
                elif isinstance(item, torch.Tensor):
                    tensors.append(item)

            # Filter by device if specified
            if args.device:
                tensors = [t for t in tensors if str(t.device).startswith(args.device)]

            if args.limit:
                tensors = tensors[: args.limit]

            if not tensors:
                msg = "No PyTorch tensors found"
                if args.device:
                    msg += f" on device '{args.device}'"
                print(msg + ".")
                return

            # Calculate total memory usage
            total_bytes = sum(t.numel() * t.element_size() for t in tensors)
            total_mb = total_bytes / (1024 * 1024)

            output = [f"Found {len(tensors)} tensor(s) using ~{total_mb:.2f} MB:", ""]

            for i, tensor in enumerate(tensors, 1):
                shape = tuple(tensor.shape)
                dtype = str(tensor.dtype).replace("torch.", "")
                device = str(tensor.device)
                size_mb = (tensor.numel() * tensor.element_size()) / (1024 * 1024)

                output.append(
                    f"  {i}. shape={shape} dtype={dtype} device={device} "
                    f"size={size_mb:.2f}MB"
                )

            print("\n".join(output))
        except ImportError:
            print("✗ PyTorch is not available")
        except Exception as e:
            print(f"✗ Failed to get torch tensors: {e}")

    def _list_optimizers(self, args):
        """List PyTorch optimizers."""
        try:
            from probing.inspect import get_torch_optimizers
            import torch

            optim_items = get_torch_optimizers()

            # Filter out non-optimizer items and extract actual objects
            optimizers = []
            for item in optim_items:
                if isinstance(item, dict) and "value" in item:
                    opt = item["value"]
                    # Skip if it's not actually an optimizer
                    if not isinstance(opt, torch.optim.Optimizer):
                        continue
                    optimizers.append((opt, item.get("id", id(opt))))
                elif isinstance(item, torch.optim.Optimizer):
                    optimizers.append((item, id(item)))

            if args.limit:
                optimizers = optimizers[: args.limit]

            if not optimizers:
                print("No PyTorch optimizers found in memory.")
                return

            output = [f"Found {len(optimizers)} optimizer(s):"]
            for i, (opt, opt_id) in enumerate(optimizers, 1):
                opt_type = type(opt).__name__
                num_param_groups = (
                    len(opt.param_groups) if hasattr(opt, "param_groups") else "?"
                )
                output.append(
                    f"  {i}. {opt_type} (id: {opt_id}, "
                    f"param_groups: {num_param_groups})"
                )

            print("\n".join(output))
        except ImportError:
            print("✗ PyTorch is not available")
        except Exception as e:
            print(f"✗ Failed to get torch optimizers: {e}")

    def _list_objects(self, args):
        """List objects in memory grouped by type."""
        import gc
        import sys
        from collections import defaultdict

        # Collect all objects
        objects = gc.get_objects()

        # Filter by type if specified
        if args.type:
            objects = [
                obj
                for obj in objects
                if type(obj).__name__ == args.type
                or f"{type(obj).__module__}.{type(obj).__name__}" == args.type
            ]

        # Group by type and count
        type_counts = defaultdict(int)
        type_sizes = defaultdict(int)

        for obj in objects:
            type_name = f"{type(obj).__module__}.{type(obj).__name__}"
            type_counts[type_name] += 1
            try:
                type_sizes[type_name] += sys.getsizeof(obj)
            except:
                pass

        # Sort by count
        sorted_types = sorted(type_counts.items(), key=lambda x: x[1], reverse=True)

        if args.limit:
            sorted_types = sorted_types[: args.limit]

        output = [f"Object types in memory (showing top {len(sorted_types)}):"]
        output.append("")
        output.append(f"{'Type':<50} {'Count':>10} {'Size (MB)':>12}")
        output.append("-" * 75)

        for type_name, count in sorted_types:
            size_mb = type_sizes[type_name] / (1024 * 1024)
            output.append(f"{type_name:<50} {count:>10} {size_mb:>12.2f}")

        print("\n".join(output))

    def _handle_gc(self):
        """Handle garbage collection."""
        import gc

        before = len(gc.get_objects())
        collected = gc.collect()
        after = len(gc.get_objects())

        print(
            f"Garbage collection complete:\n"
            f"  Objects before: {before:,}\n"
            f"  Objects after:  {after:,}\n"
            f"  Collected:      {collected:,}\n"
            f"  Freed:          {before - after:,}"
        )

    def _handle_cuda(self):
        """Handle CUDA memory inspection."""
        try:
            import torch

            if not torch.cuda.is_available():
                print("CUDA is not available on this system.")
                return

            output = ["CUDA Memory Status:"]
            output.append("")

            for i in range(torch.cuda.device_count()):
                device_name = torch.cuda.get_device_name(i)
                allocated = torch.cuda.memory_allocated(i) / (1024**3)
                reserved = torch.cuda.memory_reserved(i) / (1024**3)
                max_allocated = torch.cuda.max_memory_allocated(i) / (1024**3)

                output.append(f"Device {i}: {device_name}")
                output.append(f"  Allocated:     {allocated:.2f} GB")
                output.append(f"  Reserved:      {reserved:.2f} GB")
                output.append(f"  Max allocated: {max_allocated:.2f} GB")
                output.append("")

            print("\n".join(output))
        except ImportError:
            print("✗ PyTorch is not available")
        except Exception as e:
            print(f"✗ Failed to get CUDA memory info: {e}")

    # Aliases for convenience
    @line_magic
    def gc_collect(self, line: str):
        """Force garbage collection. Alias for %inspect gc."""
        self.inspect("gc")

    @line_magic
    def cuda_memory(self, line: str):
        """Show CUDA memory usage. Alias for %inspect cuda."""
        self.inspect("cuda")

"""IPython magic command for showing help and available commands.

This module provides a help system that uses introspection to
automatically discover all registered magic commands.
"""

from IPython.core.magic import Magics, magics_class, line_magic
from probing.repl import register_magic


@register_magic("cmds")
@magics_class
class HelpMagic(Magics):
    """Magic commands for help and documentation."""

    @line_magic
    def cmds(self, line: str):
        """List all available magic commands using introspection.

        Usage:
            %cmds                  # Show probing magic commands
            %cmds --all             # Include IPython built-in magics

        For detailed help on a specific command, use: %command?
        """
        show_all = "--all" in line or "-a" in line

        # Get all registered magics from the shell
        line_magics = self.shell.magics_manager.magics.get("line", {})
        cell_magics = self.shell.magics_manager.magics.get("cell", {})

        # Group magics by their class
        magic_groups = {}

        # Process line magics
        for name, func in line_magics.items():
            try:
                # Handle MagicAlias and bound methods
                if hasattr(func, "__self__"):
                    magic_obj = func.__self__
                elif hasattr(func, "obj"):
                    magic_obj = func.obj
                else:
                    continue

                # Filter probing magics by module path
                module = magic_obj.__class__.__module__
                if not show_all and "probing" not in module:
                    continue

                class_name = magic_obj.__class__.__name__
                if class_name not in magic_groups:
                    magic_groups[class_name] = {"line": [], "cell": []}

                # Extract description and subcommands from docstring
                doc = func.__doc__ or "No description"
                description = "No description"
                subcommands = []
                
                # Parse docstring to extract description and subcommands
                in_usage = False
                for doc_line in doc.strip().split("\n"):
                    doc_line = doc_line.strip()
                    
                    # Detect Usage section
                    if doc_line.startswith("Usage:"):
                        in_usage = True
                        continue
                    
                    # Extract subcommands from Usage section
                    if in_usage:
                        # Stop at Examples or other sections
                        if doc_line.startswith("Examples:") or doc_line.startswith("Subcommands:"):
                            in_usage = False
                            continue
                        
                        # Skip empty lines (but continue in usage mode if we have subcommands)
                        if not doc_line:
                            if subcommands:
                                in_usage = False  # End of usage section
                            continue
                        
                        # Skip comment-only lines
                        if doc_line.startswith("#"):
                            continue
                        
                        # Check if this line contains the command name
                        cmd_patterns = [f"%{name}", f"%%{name}", name]
                        subcmd_line = None
                        
                        for pattern in cmd_patterns:
                            if pattern in doc_line:
                                # Extract everything after the command name
                                parts = doc_line.split(pattern, 1)
                                if len(parts) > 1:
                                    subcmd_line = parts[1].strip()
                                    # Remove inline comments (everything after #)
                                    if "#" in subcmd_line:
                                        subcmd_line = subcmd_line.split("#", 1)[0].strip()
                                    break
                        
                        # If no command pattern found, but line looks like a subcommand (starts with common subcommands)
                        if not subcmd_line and doc_line and not doc_line.startswith("  "):
                            # Might be a subcommand line without the main command prefix
                            # Check if it looks like a subcommand (has common patterns)
                            if any(word in doc_line.lower() for word in ["watch", "list", "profile", "summary", "timeline", "ls", "gc", "cuda"]):
                                # Remove comments
                                subcmd_line = doc_line.split("#", 1)[0].strip()
                        
                        if subcmd_line:
                            subcommands.append(subcmd_line)
                    else:
                        # Extract first non-empty, non-usage, non-:: line as description
                        if (
                            doc_line
                            and not doc_line.startswith("Usage:")
                            and doc_line != "::"
                            and not doc_line.startswith("%")
                            and description == "No description"
                        ):
                            description = doc_line

                magic_groups[class_name]["line"].append((name, description, subcommands))
            except (AttributeError, KeyError):
                # Skip magics that can't be introspected
                pass

        # Process cell magics
        for name, func in cell_magics.items():
            try:
                # Handle MagicAlias and bound methods
                if hasattr(func, "__self__"):
                    magic_obj = func.__self__
                elif hasattr(func, "obj"):
                    magic_obj = func.obj
                else:
                    continue

                module = magic_obj.__class__.__module__
                if not show_all and "probing" not in module:
                    continue

                class_name = magic_obj.__class__.__name__
                if class_name not in magic_groups:
                    magic_groups[class_name] = {"line": [], "cell": []}

                doc = func.__doc__ or "No description"
                description = "No description"
                subcommands = []
                
                # Parse docstring to extract description and subcommands
                in_usage = False
                for doc_line in doc.strip().split("\n"):
                    doc_line = doc_line.strip()
                    
                    # Detect Usage section
                    if doc_line.startswith("Usage:"):
                        in_usage = True
                        continue
                    
                    # Extract subcommands from Usage section
                    if in_usage:
                        # Stop at Examples or other sections
                        if doc_line.startswith("Examples:") or doc_line.startswith("Subcommands:"):
                            in_usage = False
                            continue
                        
                        # Skip empty lines
                        if not doc_line:
                            if subcommands:
                                in_usage = False
                            continue
                        
                        # Skip comment-only lines
                        if doc_line.startswith("#"):
                            continue
                        
                        # Check if this line contains the command name
                        cmd_patterns = [f"%%{name}", f"%{name}", name]
                        subcmd_line = None
                        
                        for pattern in cmd_patterns:
                            if pattern in doc_line:
                                parts = doc_line.split(pattern, 1)
                                if len(parts) > 1:
                                    subcmd_line = parts[1].strip()
                                    # Remove inline comments
                                    if "#" in subcmd_line:
                                        subcmd_line = subcmd_line.split("#", 1)[0].strip()
                                    break
                        
                        if subcmd_line:
                            subcommands.append(subcmd_line)
                    else:
                        # Extract first non-empty line as description
                        if (
                            doc_line
                            and not doc_line.startswith("Usage:")
                            and doc_line != "::"
                            and not doc_line.startswith("%")
                            and description == "No description"
                        ):
                            description = doc_line

                magic_groups[class_name]["cell"].append((name, description, subcommands))
            except (AttributeError, KeyError):
                # Skip magics that can't be introspected
                pass

        # Build output
        title = "🔮 Probing Magic Commands" if not show_all else "🔮 All Magic Commands"
        output = [title, "=" * 70, ""]

        for class_name in sorted(magic_groups.keys()):
            group = magic_groups[class_name]

            # Extract nice name from class (e.g., QueryMagic -> Query)
            display_name = class_name.replace("Magic", "")
            output.append(f"📦 {display_name}")
            output.append("-" * 70)

            # Show line magics
            for item in sorted(group["line"]):
                if len(item) == 3:
                    name, desc, subcommands = item
                else:
                    # Backward compatibility
                    name, desc = item[:2]
                    subcommands = []
                
                # Truncate long descriptions
                desc_short = desc[:50] + "..." if len(desc) > 50 else desc
                output.append(f"  %{name:<25} {desc_short}")
                
                # Show subcommands if available
                if subcommands:
                    for subcmd in subcommands[:5]:  # Limit to 5 subcommands
                        # Clean up subcommand line
                        subcmd_clean = subcmd.strip()
                        # Remove leading # if present
                        if subcmd_clean.startswith("#"):
                            subcmd_clean = subcmd_clean[1:].strip()
                        # Truncate if too long
                        if len(subcmd_clean) > 60:
                            subcmd_clean = subcmd_clean[:57] + "..."
                        output.append(f"    └─ %{name} {subcmd_clean}")
                    if len(subcommands) > 5:
                        output.append(f"    └─ ... and {len(subcommands) - 5} more (use %{name}? for full help)")

            # Show cell magics
            for item in sorted(group["cell"]):
                if len(item) == 3:
                    name, desc, subcommands = item
                else:
                    name, desc = item[:2]
                    subcommands = []
                
                desc_short = desc[:50] + "..." if len(desc) > 50 else desc
                output.append(f"  %%{name:<24} {desc_short}")
                
                # Show subcommands if available
                if subcommands:
                    for subcmd in subcommands[:5]:
                        subcmd_clean = subcmd.strip()
                        if subcmd_clean.startswith("#"):
                            subcmd_clean = subcmd_clean[1:].strip()
                        if len(subcmd_clean) > 60:
                            subcmd_clean = subcmd_clean[:57] + "..."
                        output.append(f"    └─ %%{name} {subcmd_clean}")
                    if len(subcommands) > 5:
                        output.append(f"    └─ ... and {len(subcommands) - 5} more (use %%{name}? for full help)")

            output.append("")

        output.extend(
            [
                "💡 Tips:",
                "  • Use %command? for detailed help",
                "  • Use %%command? for cell magic help",
                "  • Use Tab for auto-completion",
            ]
        )

        if not show_all:
            output.append("  • Use %cmds --all to see all IPython magics")

        output.append("")
        output.append("=" * 70)

        print("\n".join(output))

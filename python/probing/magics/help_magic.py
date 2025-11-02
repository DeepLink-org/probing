"""IPython magic command for showing help and available commands.

This module provides a help system that uses introspection to
automatically discover all registered magic commands.
"""

from IPython.core.magic import Magics, magics_class, line_magic


@magics_class
class HelpMagic(Magics):
    """Magic commands for help and documentation."""

    @line_magic
    def lsmagics(self, line: str):
        """List all available magic commands using introspection.

        Usage:
            %lsmagics              # Show probing magic commands
            %lsmagics --all        # Include IPython built-in magics
        
        For detailed help on a specific command, use: %command?
        """
        show_all = '--all' in line or '-a' in line
        
        # Get all registered magics from the shell
        line_magics = self.shell.magics_manager.magics.get('line', {})
        cell_magics = self.shell.magics_manager.magics.get('cell', {})
        
        # Group magics by their class
        magic_groups = {}
        
        # Process line magics
        for name, func in line_magics.items():
            try:
                # Handle MagicAlias and bound methods
                if hasattr(func, '__self__'):
                    magic_obj = func.__self__
                elif hasattr(func, 'obj'):
                    magic_obj = func.obj
                else:
                    continue
                
                # Filter probing magics by module path
                module = magic_obj.__class__.__module__
                if not show_all and 'probing' not in module:
                    continue
                
                class_name = magic_obj.__class__.__name__
                if class_name not in magic_groups:
                    magic_groups[class_name] = {'line': [], 'cell': []}
                
                # Extract first line of docstring as description
                doc = func.__doc__ or "No description"
                # Get first non-empty, non-usage, non-:: line
                for doc_line in doc.strip().split('\n'):
                    doc_line = doc_line.strip()
                    # Skip empty lines, Usage lines, :: (magic_arguments marker), and % lines (auto-generated usage)
                    if doc_line and not doc_line.startswith('Usage:') and doc_line != '::' and not doc_line.startswith('%'):
                        description = doc_line
                        break
                else:
                    description = "No description"
                
                magic_groups[class_name]['line'].append((name, description))
            except (AttributeError, KeyError):
                # Skip magics that can't be introspected
                pass
        
        # Process cell magics
        for name, func in cell_magics.items():
            try:
                # Handle MagicAlias and bound methods
                if hasattr(func, '__self__'):
                    magic_obj = func.__self__
                elif hasattr(func, 'obj'):
                    magic_obj = func.obj
                else:
                    continue
                
                module = magic_obj.__class__.__module__
                if not show_all and 'probing' not in module:
                    continue
                
                class_name = magic_obj.__class__.__name__
                if class_name not in magic_groups:
                    magic_groups[class_name] = {'line': [], 'cell': []}
                
                doc = func.__doc__ or "No description"
                # Get first non-empty, non-usage, non-:: line
                for doc_line in doc.strip().split('\n'):
                    doc_line = doc_line.strip()
                    # Skip empty lines, Usage lines, :: (magic_arguments marker), and % lines (auto-generated usage)
                    if doc_line and not doc_line.startswith('Usage:') and doc_line != '::' and not doc_line.startswith('%'):
                        description = doc_line
                        break
                else:
                    description = "No description"
                
                magic_groups[class_name]['cell'].append((name, description))
            except (AttributeError, KeyError):
                # Skip magics that can't be introspected
                pass
        
        # Build output
        title = "🔮 Probing Magic Commands" if not show_all else "🔮 All Magic Commands"
        output = [title, "=" * 70, ""]
        
        for class_name in sorted(magic_groups.keys()):
            group = magic_groups[class_name]
            
            # Extract nice name from class (e.g., QueryMagic -> Query)
            display_name = class_name.replace('Magic', '')
            output.append(f"📦 {display_name}")
            output.append("-" * 70)
            
            # Show line magics
            for name, desc in sorted(group['line']):
                # Truncate long descriptions
                desc = desc[:50] + "..." if len(desc) > 50 else desc
                output.append(f"  %{name:<25} {desc}")
            
            # Show cell magics
            for name, desc in sorted(group['cell']):
                desc = desc[:50] + "..." if len(desc) > 50 else desc
                output.append(f"  %%{name:<24} {desc}")
            
            output.append("")
        
        output.extend([
            "💡 Tips:",
            "  • Use %command? for detailed help",
            "  • Use %%command? for cell magic help",
            "  • Use Tab for auto-completion",
        ])
        
        if not show_all:
            output.append("  • Use %lsmagics --all to see all IPython magics")
        
        output.append("")
        output.append("=" * 70)
        
        print("\n".join(output))

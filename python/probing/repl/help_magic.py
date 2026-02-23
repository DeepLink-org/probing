"""IPython magic command for showing help and available commands.

This module provides a help system that uses introspection to
automatically discover all registered magic commands.

All metadata (label, command, help) is derived from reflection:
- @magic_arguments / @argument decorators: primary source for magics using them
- Docstring Usage section: fallback for subcommand syntax and inline help
- Subcommand handler methods: _cmd_{magic}_{subcmd} or _cmd_{subcmd} docstrings
- Magic class/function docstring: overall description
"""

import re
from typing import Any, Dict, List, Optional, Tuple

from IPython.core.magic import Magics, line_magic, magics_class

from probing.repl import register_magic


def _parse_choices_from_help(help_str: Optional[str]) -> List[str]:
    """Extract choice tokens from argument help, e.g. 'Subcommand: ls/list, gc, cuda' -> ['ls','list','gc','cuda']."""
    if not help_str or not help_str.strip():
        return []
    text = help_str
    if ":" in text:
        text = text.split(":", 1)[1].strip()
    parts: List[str] = []
    for segment in re.split(r",|\s+or\s+", text, flags=re.I):
        segment = segment.strip()
        if not segment:
            continue
        if "/" in segment:
            for alt in segment.split("/"):
                a = alt.strip()
                if a and a not in parts:
                    parts.append(a)
        else:
            if segment not in parts:
                parts.append(segment)
    return parts


def _introspect_magic_arguments(func: Any) -> Optional[List[Tuple[str, str]]]:
    """Extract subcommands from @magic_arguments / @argument decorator metadata.

    Returns list of (command_suffix, help_text) e.g. [('ls modules', '...'), ('gc', '...')],
    or None if func has no parser.
    """
    parser = getattr(func, "parser", None)
    if parser is None or not hasattr(parser, "_actions"):
        return None

    positionals: List[Tuple[str, str]] = []
    for action in parser._actions:
        if action.dest == "help":
            continue
        if getattr(action, "option_strings", None):
            continue
        dest = getattr(action, "dest", None)
        help_str = getattr(action, "help", None) or ""
        if dest:
            positionals.append((dest, help_str))

    if not positionals:
        return [("", getattr(parser, "description", None) or "")]

    choices_per_pos: List[List[str]] = []
    help_per_pos: List[str] = []
    for dest, help_str in positionals:
        choices = _parse_choices_from_help(help_str)
        choices_per_pos.append(choices if choices else [""])
        help_per_pos.append(help_str)

    if len(positionals) == 1:
        if choices_per_pos[0]:
            return [
                (c, help_per_pos[0]) for c in choices_per_pos[0] if c and c != "help"
            ]
        return [("", help_per_pos[0])]

    subcmd_choices = choices_per_pos[0]
    target_choices = choices_per_pos[1] if len(choices_per_pos) > 1 else []

    excludes = {"help"}
    subcmd_choices = [c for c in subcmd_choices if c and c not in excludes]

    result: List[Tuple[str, str]] = []
    sub_added: set = set()
    for sub in subcmd_choices:
        cmd_sub = "ls" if sub == "list" else sub
        if target_choices and sub in ("ls", "list"):
            if "ls" in sub_added:
                continue
            sub_added.add("ls")
            for t in target_choices:
                if t:
                    result.append((f"ls {t}", help_per_pos[1] or help_per_pos[0]))
        else:
            if cmd_sub in sub_added:
                continue
            sub_added.add(cmd_sub)
            result.append((cmd_sub, help_per_pos[0]))
    return result if result else None


def _split_subcmd_help(line: str) -> Tuple[str, str]:
    """Split 'watch    Show currently watched' into ('watch', 'Show currently watched')."""
    m = re.match(r"^(.+?)\s{2,}(.+)$", line)
    if m:
        return m.group(1).strip(), m.group(2).strip()
    return line.strip(), ""


def _extract_invokable(syntax: str) -> str:
    """Extract runnable part from syntax, stripping optional args.
    'list [<prefix>] [--limit <n>]' -> 'list'
    'ls modules --all' -> 'ls modules'
    'profile [steps=N]' -> 'profile'
    """
    tokens = []
    for tok in syntax.split():
        if tok.startswith("[") or tok.startswith("<") or tok.startswith("--"):
            break
        tokens.append(tok)
    return " ".join(tokens) if tokens else syntax.split()[0] if syntax.split() else ""


def _get_subcommand_method_doc(
    magic_obj: Any, magic_name: str, subcmd_base: str
) -> str:
    """Try to get docstring from subcommand handler via reflection."""
    if not subcmd_base:
        return ""
    subcmd_underscore = subcmd_base.replace(" ", "_").replace("-", "_")
    first_word = subcmd_base.split()[0] if subcmd_base else ""
    for method_name in (
        f"_cmd_{magic_name}_{subcmd_underscore}",
        f"_cmd_{magic_name}_{first_word}",
        f"_cmd_{subcmd_underscore}",
        f"_handle_{first_word}",
        f"_handle_{subcmd_underscore}",
    ):
        if method_name and hasattr(magic_obj, method_name):
            m = getattr(magic_obj, method_name)
            if callable(m) and m.__doc__:
                first_line = m.__doc__.strip().split("\n")[0]
                return first_line.strip()
    return ""


def _discover_subcommands_from_class(
    magic_obj: Any, magic_name: str
) -> List[Tuple[str, str]]:
    """Discover subcommands by reflecting on magic class methods.
    Finds _cmd_{name}_{subcmd}, _handle_{subcmd} etc. and derives (syntax, help) from docstrings.
    """
    result: List[Tuple[str, str]] = []
    seen: set = set()
    for attr in dir(magic_obj):
        if not attr.startswith("_") or attr.startswith("__"):
            continue
        subcmd = None
        if attr.startswith(f"_cmd_{magic_name}_"):
            subcmd = attr[len(f"_cmd_{magic_name}_") :].replace("_", " ")
        elif attr.startswith("_handle_"):
            subcmd = attr[len("_handle_") :].replace("_", " ")
        elif attr.startswith("_cmd_") and magic_name not in attr:
            subcmd = attr[len("_cmd_") :].replace("_", " ")
        if subcmd and subcmd not in seen:
            seen.add(subcmd)
            m = getattr(magic_obj, attr)
            help_txt = (m.__doc__ or "").strip().split("\n")[0] if callable(m) else ""
            result.append((subcmd, help_txt))
    return result


def _build_magic_groups(
    shell: Any, show_all: bool = False
) -> Dict[str, Dict[str, List]]:
    """Build magic_groups dict from shell introspection. Shared by cmds() and get_magics_for_ui()."""
    line_magics = shell.magics_manager.magics.get("line", {})
    cell_magics = shell.magics_manager.magics.get("cell", {})
    magic_groups: Dict[str, Dict[str, List]] = {}

    def process_magics(magics_dict: dict, magic_type: str) -> None:
        for name, func in magics_dict.items():
            try:
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

                subcommands: List[Tuple[str, str]] = []
                parser_subcmds = _introspect_magic_arguments(func)
                if parser_subcmds:
                    subcommands = parser_subcmds
                else:
                    subcommands = []

                doc = func.__doc__ or "No description"
                if parser_subcmds:
                    parser = getattr(func, "parser", None)
                    description = (
                        (getattr(parser, "description", None) or "").strip()
                        or doc.split("\n")[0].strip()
                        or "No description"
                    )
                else:
                    description = "No description"
                    in_usage = False
                    for doc_line in doc.strip().split("\n"):
                        doc_line = doc_line.strip()
                        if doc_line.startswith("Usage:"):
                            in_usage = True
                            continue
                        if in_usage:
                            if doc_line.startswith("Examples:") or doc_line.startswith(
                                "Subcommands:"
                            ):
                                in_usage = False
                                continue
                            if not doc_line:
                                if subcommands:
                                    in_usage = False
                                continue
                            if doc_line.startswith("#"):
                                continue

                            cmd_patterns = [f"%{name}", f"%%{name}", name]
                            subcmd_line = None
                            for pattern in cmd_patterns:
                                if pattern in doc_line:
                                    parts = doc_line.split(pattern, 1)
                                    if len(parts) > 1:
                                        subcmd_line = parts[1].strip()
                                        if "#" in subcmd_line:
                                            subcmd_line = subcmd_line.split("#", 1)[
                                                0
                                            ].strip()
                                        break

                            if subcmd_line:
                                syntax, help_text = _split_subcmd_help(subcmd_line)
                                subcommands.append((syntax, help_text))
                        else:
                            if (
                                doc_line
                                and not doc_line.startswith("Usage:")
                                and doc_line != "::"
                                and not doc_line.startswith("%")
                                and description == "No description"
                            ):
                                description = doc_line

                magic_groups[class_name][magic_type].append(
                    (name, description, subcommands, magic_obj)
                )
            except (AttributeError, KeyError):
                pass

    process_magics(line_magics, "line")
    process_magics(cell_magics, "cell")
    return magic_groups


def _label_from_help(help_text: str, fallback_base: str) -> str:
    """Derive UI label from help text or subcommand name."""
    if help_text:
        # Use first ~30 chars of help, or first sentence
        first = help_text.split(".")[0].strip()
        if len(first) <= 35:
            return first
        return first[:32].rsplit(" ", 1)[0] + "..."
    return fallback_base.replace("_", " ").replace("-", " ").title()


def get_magics_for_ui(shell: Any) -> List[Dict[str, Any]]:
    """Return magics as JSON-serializable list for UI quick actions.

    All data is derived from introspection (docstrings, method reflection).
    Returns:
        [{"group": "Trace", "items": [{"label": "...", "command": "%trace list", "help": "..."}, ...]}, ...]
    """
    magic_groups = _build_magic_groups(shell, show_all=False)
    result: List[Dict[str, Any]] = []

    for class_name in sorted(magic_groups.keys()):
        group = magic_groups[class_name]
        display_name = class_name.replace("Magic", "")
        items: List[Dict[str, str]] = []
        seen_commands: set = set()

        for item in group["line"]:
            if len(item) >= 4:
                name, _desc, subcommands, magic_obj = item[0], item[1], item[2], item[3]
            elif len(item) == 3:
                name, _desc, subcommands = item
                magic_obj = None
            else:
                name, _desc = item[:2]
                subcommands = []
                magic_obj = None

            def add_item(label: str, command: str, help_text: str) -> None:
                if command and command not in seen_commands:
                    seen_commands.add(command)
                    items.append(
                        {"label": label, "command": command, "help": help_text or ""}
                    )

            prefix = f"%{name} "
            if not subcommands and magic_obj:
                subcommands = _discover_subcommands_from_class(magic_obj, name)
            for subcmd_entry in subcommands:
                subcmd_raw = (
                    subcmd_entry[0] if isinstance(subcmd_entry, tuple) else subcmd_entry
                )
                docstring_help = (
                    subcmd_entry[1]
                    if isinstance(subcmd_entry, tuple) and len(subcmd_entry) > 1
                    else ""
                )
                subcmd_clean = (
                    subcmd_raw.strip()
                    if isinstance(subcmd_raw, str)
                    else str(subcmd_raw)
                )
                if subcmd_clean.startswith("<"):
                    continue

                invokable = _extract_invokable(subcmd_clean)
                if not invokable:
                    if subcmd_clean == "" and docstring_help:
                        command = f"%{name}"
                        help_txt = docstring_help
                        label = _label_from_help(help_txt, display_name)
                        add_item(label, command, help_txt)
                    continue
                base = invokable.split()[0] if invokable else subcmd_clean.split()[0]

                command = f"{prefix}{invokable}"
                help_txt = (
                    _get_subcommand_method_doc(magic_obj, name, invokable)
                    if magic_obj
                    else ""
                )
                if not help_txt:
                    help_txt = docstring_help
                label = _label_from_help(help_txt, invokable.replace("_", " "))
                add_item(label, command, help_txt)

            # Magics with no subcommands (e.g. %tables, %cmds)
            if not subcommands:
                command = f"%{name}"
                help_txt = _desc if _desc and _desc != "No description" else ""
                label = _label_from_help(help_txt, display_name)
                add_item(label, command, help_txt)

        if items:
            result.append({"group": display_name, "items": items})

    return result


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
        magic_groups = _build_magic_groups(self.shell, show_all=show_all)

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
            for item in sorted(group["line"], key=lambda x: x[0] if x else ""):
                if len(item) >= 3:
                    name, desc, subcommands = item[0], item[1], item[2]
                else:
                    name, desc = item[:2]
                    subcommands = []

                # Truncate long descriptions
                desc_short = desc[:50] + "..." if len(desc) > 50 else desc
                output.append(f"  %{name:<25} {desc_short}")

                # Show subcommands if available
                if subcommands:
                    for subcmd in subcommands[:5]:  # Limit to 5 subcommands
                        syntax = subcmd[0] if isinstance(subcmd, tuple) else subcmd
                        help_txt = (
                            subcmd[1]
                            if isinstance(subcmd, tuple) and len(subcmd) > 1
                            else ""
                        )
                        syntax = (
                            syntax.strip() if isinstance(syntax, str) else str(syntax)
                        )
                        if syntax.startswith("#"):
                            syntax = syntax[1:].strip()
                        if len(syntax) > 60:
                            syntax = syntax[:57] + "..."
                        line = f"    └─ %{name} {syntax}"
                        if help_txt:
                            line += f"  # {help_txt[:40]}{'...' if len(help_txt) > 40 else ''}"
                        output.append(line)
                    if len(subcommands) > 5:
                        output.append(
                            f"    └─ ... and {len(subcommands) - 5} more (use %{name}? for full help)"
                        )

            # Show cell magics
            for item in sorted(group["cell"], key=lambda x: x[0] if x else ""):
                if len(item) >= 3:
                    name, desc, subcommands = item[0], item[1], item[2]
                else:
                    name, desc = item[:2]
                    subcommands = []

                desc_short = desc[:50] + "..." if len(desc) > 50 else desc
                output.append(f"  %%{name:<24} {desc_short}")

                # Show subcommands if available
                if subcommands:
                    for subcmd in subcommands[:5]:
                        syntax = subcmd[0] if isinstance(subcmd, tuple) else subcmd
                        help_txt = (
                            subcmd[1]
                            if isinstance(subcmd, tuple) and len(subcmd) > 1
                            else ""
                        )
                        syntax = (
                            syntax.strip() if isinstance(syntax, str) else str(syntax)
                        )
                        if syntax.startswith("#"):
                            syntax = syntax[1:].strip()
                        if len(syntax) > 60:
                            syntax = syntax[:57] + "..."
                        line = f"    └─ %%{name} {syntax}"
                        if help_txt:
                            line += f"  # {help_txt[:40]}{'...' if len(help_txt) > 40 else ''}"
                        output.append(line)
                    if len(subcommands) > 5:
                        output.append(
                            f"    └─ ... and {len(subcommands) - 5} more (use %%{name}? for full help)"
                        )

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

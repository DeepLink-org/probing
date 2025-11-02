"""IPython magic commands for function tracing.

This module provides magic commands to trace Python function execution,
watch variables, and monitor function calls.
"""

from IPython.core.magic import Magics, magics_class, line_magic, cell_magic
from IPython.core.magic_arguments import argument, magic_arguments, parse_argstring
from probing.magics import register_magic
import json


@register_magic("trace")
@magics_class
class TraceMagic(Magics):
    """Magic commands for tracing function execution."""

    @line_magic
    @magic_arguments()
    @argument('function', type=str, help='Function name to trace (e.g., torch.nn.Module.forward)')
    @argument('--watch', '-w', nargs='+', default=[], help='Variables to watch')
    @argument('--depth', '-d', type=int, default=1, help='Tracing depth')
    def trace(self, line: str):
        """Start tracing a function.

        Usage:
            %trace torch.nn.Linear.forward --watch input output --depth 2
            %trace mymodule.myfunction
        """
        from probing.trace import trace as trace_func
        
        args = parse_argstring(self.trace, line)
        
        try:
            trace_func(args.function, watch=args.watch, depth=args.depth)
            return f"✓ Started tracing: {args.function}"
        except Exception as e:
            return f"✗ Failed to trace {args.function}: {e}"

    @line_magic
    @magic_arguments()
    @argument('function', type=str, help='Function name to untrace')
    def untrace(self, line: str):
        """Stop tracing a function.

        Usage:
            %untrace torch.nn.Linear.forward
        """
        from probing.trace import untrace as untrace_func
        
        args = parse_argstring(self.untrace, line)
        
        try:
            untrace_func(args.function)
            return f"✓ Stopped tracing: {args.function}"
        except Exception as e:
            return f"✗ Failed to untrace {args.function}: {e}"

    @line_magic
    def show_trace(self, line: str):
        """Show currently traced functions.

        Usage:
            %show_trace
        """
        from probing.trace import show_trace
        
        result = show_trace()
        traced = json.loads(result)
        
        if not traced:
            return "No functions are currently being traced."
        
        output = ["Currently traced functions:"]
        for i, func in enumerate(traced, 1):
            output.append(f"  {i}. {func}")
        return "\n".join(output)

    @line_magic
    @magic_arguments()
    @argument('--prefix', '-p', type=str, default=None, help='Filter by prefix')
    def list_traceable(self, line: str):
        """List all traceable functions.

        Usage:
            %list_traceable --prefix torch.nn
            %list_traceable -p torch.optim
        """
        from probing.trace import list_traceable as list_traceable_func
        
        args = parse_argstring(self.list_traceable, line)
        
        result = list_traceable_func(prefix=args.prefix)
        functions = json.loads(result)
        
        if not functions:
            return f"No traceable functions found with prefix: {args.prefix}"
        
        # Limit output to avoid overwhelming the terminal
        max_display = 50
        output = [f"Found {len(functions)} traceable functions"]
        if args.prefix:
            output[0] += f" with prefix '{args.prefix}'"
        output.append("")
        
        for i, func in enumerate(functions[:max_display], 1):
            output.append(f"  {i}. {func}")
        
        if len(functions) > max_display:
            output.append(f"\n  ... and {len(functions) - max_display} more")
            output.append(f"\nTip: Use --prefix to narrow down results")
        
        return "\n".join(output)

    @cell_magic
    @magic_arguments()
    @argument('--watch', '-w', nargs='+', default=[], help='Variables to watch')
    @argument('--depth', '-d', type=int, default=1, help='Tracing depth')
    def probe(self, line: str, cell: str):
        """Execute code with probing enabled.

        Usage:
            %%probe --watch x y --depth 2
            def my_function(x):
                y = x * 2
                return y
            
            result = my_function(5)
        """
        from probing.trace import probe as probe_decorator
        
        args = parse_argstring(self.probe, line)
        
        # Execute the cell code in the user's namespace
        # with probing enabled for all functions
        exec_code = f"""
from probing.trace import probe as _probe_decorator

# Wrap execution in probe context
_probing_tracer = _probe_decorator(watch={args.watch!r}, depth={args.depth})
with _probing_tracer:
{chr(10).join('    ' + line for line in cell.split(chr(10)))}
"""
        
        self.shell.run_cell(exec_code)

"""IPython magic commands for querying data and managing extensions.

This module provides magic commands to execute queries and manage
extensions in the probing system.
"""

from IPython.core.magic import Magics, magics_class, line_magic
from IPython.core.magic_arguments import argument, magic_arguments, parse_argstring
from IPython.display import display
from probing.repl import register_magic
import pandas as pd


@register_magic("query")
@magics_class
class QueryMagic(Magics):
    """Magic commands for querying data and managing extensions."""

    @line_magic
    def query(self, line: str):
        """Execute a query and return results as DataFrame.

        Usage:
            %query SELECT * FROM my_table LIMIT 10
            %query SHOW TABLES
            %query DESCRIBE my_table

        Short form:
            %q SELECT * FROM my_table LIMIT 10
        """
        from probing.core.engine import query as query_func

        if not line.strip():
            return "Error: Query cannot be empty"

        try:
            result = query_func(line)
            if isinstance(result, pd.DataFrame):
                display(result)
                return result
            else:
                print(result)
                return result
        except Exception as e:
            return f"✗ Query failed: {e}"

    @line_magic
    def q(self, line: str):
        """Short alias for %query command.

        Usage:
            %q SELECT * FROM my_table LIMIT 10
            %q SHOW TABLES
        """
        return self.query(line)

    @line_magic
    @magic_arguments()
    @argument("extension", type=str, help="Extension module to load")
    def load_ext(self, line: str):
        """Load a probing extension.

        Usage:
            %load_ext probing.ext.example
            %load_ext probing.ext.torch
        """
        from probing.core.engine import load_extension

        args = parse_argstring(self.load_ext, line)

        try:
            result = load_extension(args.extension)
            return f"✓ Extension loaded: {args.extension}"
        except Exception as e:
            return f"✗ Failed to load extension {args.extension}: {e}"

    @line_magic
    def tables(self, line: str):
        """Show all available tables.

        Usage:
            %tables
        """
        from probing.core.engine import query as query_func

        try:
            result = query_func("SHOW TABLES")
            if isinstance(result, pd.DataFrame):
                display(result)
                return result
            else:
                print(result)
                return result
        except Exception as e:
            return f"✗ Failed to show tables: {e}"

    @line_magic
    @magic_arguments()
    @argument("table_name", type=str, help="Table name to describe")
    def describe(self, line: str):
        """Describe the schema of a table.

        Usage:
            %describe my_table
        """
        from probing.core.engine import query as query_func

        args = parse_argstring(self.describe, line)

        try:
            result = query_func(f"DESCRIBE {args.table_name}")
            if isinstance(result, pd.DataFrame):
                display(result)
                return result
            else:
                print(result)
                return result
        except Exception as e:
            return f"✗ Failed to describe table: {e}"

    @line_magic
    @magic_arguments()
    @argument("table_name", type=str, help="Table name")
    @argument("--limit", "-n", type=int, default=10, help="Number of rows to show")
    def peek(self, line: str):
        """Quick peek at table data.

        Usage:
            %peek my_table
            %peek my_table --limit 20
            %peek my_table -n 50
        """
        from probing.core.engine import query as query_func

        args = parse_argstring(self.peek, line)

        try:
            result = query_func(f"SELECT * FROM {args.table_name} LIMIT {args.limit}")
            if isinstance(result, pd.DataFrame):
                display(result)
                return result
            else:
                print(result)
                return result
        except Exception as e:
            return f"✗ Failed to peek table: {e}"

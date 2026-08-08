"""
Core Engine Module

Spec
----
This module exposes the core engine capabilities for data definition and querying.

Responsibilities:
1.  Define the `table` decorator for mapping Python classes to structural tables.
2.  Expose query execution capabilities via `query`.
3.  Provide extension loading mechanisms.

Public Interfaces:
- `table`: Decorator to define schemas.
- `query`: Execute SQL-like queries on tables.
- `load_extension`: Load dynamic extensions into the engine.
"""

from .engine import QueryOutcome, QueryQuality, load_extension, query, query_outcome
from .table import table

__all__ = [
    "table",
    "query",
    "query_outcome",
    "QueryOutcome",
    "QueryQuality",
    "load_extension",
]

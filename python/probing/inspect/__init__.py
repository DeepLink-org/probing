"""
Runtime Inspection

Spec
----
This module provides tools for inspecting and instrumenting Python objects at runtime.

Responsibilities:
1.  Provide `trace` and `untrace` APIs for dynamic function instrumentation.
2.  Provide object inspection utilities (e.g., for PyTorch Tensors, Modules, Optimizers).
3.  Bridge Python runtime data with the Probing engine.

Public Interfaces:
- `trace`/`untrace`: Dynamic function tracing.
- `probe`: Inspect an object's internal state.
- `show_trace`: Visualize trace data.
- `list_traceable`: List objects that can be traced.
- `get_torch_modules`/`tensors`/`optimizers`: PyTorch-specific inspectors.
"""

# Export trace module for convenience

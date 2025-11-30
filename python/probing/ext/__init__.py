"""
Framework Extensions

Spec
----
This package provides integrations for third-party AI frameworks.

Responsibilities:
1.  Provide automatic tracing for distributed frameworks (e.g., Ray).
2.  Provide hooks and profilers for Deep Learning frameworks (e.g., PyTorch).
3.  Normalize framework-specific events into Probing spans.

Submodules:
- `ray`: Ray task and actor tracing.
- `torch`: PyTorch profiling hooks and utilities.
"""

"""Profiling collectors, including two independent PyTorch paths.

``torch_probe`` is sampled, long-running module/step telemetry. It writes mmap
tables such as ``python.torch_trace`` and ``python.torch_step_timing``.

``torch_profiler`` controls explicit short ``torch.profiler`` / Kineto captures.
It keeps bounded capture state in process and exposes conclusion-oriented virtual
tables, ``python.profile_capture`` and ``python.profile_hotspot``.

The paths do not start, stop, or configure one another. They correlate only by
training coordinates in SQL, and their overheads add when both are active.
"""

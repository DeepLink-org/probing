from dataclasses import dataclass
from typing import Any


@dataclass
class TensorDef:
    shape: tuple = ()
    dtype: Any = None

    @staticmethod
    def create(t):
        return TensorDef(t.shape, t.dtype)


class BaseTracer:
    """Base hook dispatcher for module/optimizer instrumentation.

    Per-step execution ordering (``offset``) and the "current module/stage"
    de-duplication state are kept on the instance — never as module globals —
    so multiple tracers (e.g. one per optimizer) and concurrent training loops
    cannot corrupt each other's sequencing or per-step time anchor.

    The authoritative training step coordinate lives in Rust
    (``probing.tracing.step_snapshot``); this class no longer maintains a
    separate Python step counter.
    """

    def __init__(self, **kwargs):
        # Execution order within the current step; reset on each optimizer step.
        self._module_call_offset = 0
        # Last (module_id, stage) seen, used to advance offset only on change.
        self._current_module = None
        self._current_stage = None
        super().__init__(**kwargs)

    def offset(self):
        return self._module_call_offset

    def process_hook(self, module, stage):
        if self._current_module != id(module) or self._current_stage != stage:
            self._module_call_offset += 1
            self._current_module = id(module)
            self._current_stage = stage

    def pre_forward_hook(self, m, i):
        self.log_module_stage("pre forward", m)
        self.process_hook(m, "pre forward")

    def post_forward_hook(self, m, i, o):
        self.log_module_stage("post forward", m)
        self.process_hook(m, "post forward")

    def pre_backward_hook(self, m, i):
        self.log_module_stage("pre backward", m)
        self.process_hook(m, "pre backward")

    def post_backward_hook(self, m, i, o):
        self.log_module_stage("post backward", m)
        self.process_hook(m, "post backward")

    def pre_step_hook(self, optimizer, args, kwargs):
        self.log_module_stage("pre step", optimizer, force=True)
        self.process_hook(optimizer, "pre step")

    def post_step_hook(self, optimizer, args, kwargs):
        self.log_module_stage("post step", optimizer, force=True)
        self.process_hook(optimizer, "post step")
        # New step begins: reset intra-step execution offset. The training step
        # coordinate itself is advanced by the train-step span (Rust side).
        self._module_call_offset = 0

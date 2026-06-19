import random
import time
from dataclasses import dataclass
from typing import Optional

import probing
from probing.core import table
from probing.tracing import (
    TRAIN_STEP_KIND,
    current_local_step,
    module_stage_kind,
    recorded_span,
    sync_local_step,
)

from .torch.module_utils import module_name
from .types import BaseTracer

TRUE_VALUES = {"1", "true", "yes", "on", "enable", "enabled"}
FALSE_VALUES = {"0", "false", "no", "off", "disable", "disabled"}


# Detect and set the appropriate backend (CUDA or MPS)
def _get_backend():
    """Detect and return the appropriate PyTorch backend module."""
    import torch

    if torch.cuda.is_available():
        return torch.cuda
    elif hasattr(torch.backends, "mps") and torch.backends.mps.is_available():
        return torch.mps
    else:
        return None


backend = _get_backend()


@table
@dataclass
class TorchTrace:
    step: Optional[int] = None
    seq: Optional[int] = None
    module: Optional[str] = None
    stage: Optional[str] = None
    allocated: float = 0.0
    max_allocated: float = 0.0
    cached: float = 0.0
    max_cached: float = 0.0
    time_offset: float = 0.0
    duration: float = 0.0
    allocated_delta: float = 0.0
    max_allocated_delta: float = 0.0


@table
@dataclass
class Variables:
    step: Optional[int] = None
    func: Optional[str] = None
    name: Optional[str] = None
    value: Optional[str] = None


@dataclass
class TorchProbeConfig:
    """Configuration container for TorchProbe runtime behaviour.

    Torch profiling is designed for long-running, always-on module-level
    telemetry (not episodic ``torch.profiler`` windows). There is no warmup
    schedule: skip early steps in SQL (``WHERE step > N``) when needed.

    Sampling modes (``mode`` / ``PROBING_TORCH_PROFILING`` prefix token):

    - **ordered** — ``rate`` is the probability each training step is sampled.
      On sampled steps, one module rotates per step (``curr_mod``), plus a
      per-step time anchor: the first ``pre`` hook in the step (``offset=0``).
      Any ``pre`` hook that is recorded always gets its matching ``post`` hook.
    - **random** — every step is sampled. ``rate`` is the per-hook probability
      for ``offset > 0`` on ``pre`` hooks; matching ``post`` hooks always pair.
      The ``offset=0`` ``pre`` anchor is always recorded.

    The first complete training step is discovery only (modules registered,
    no rows written). Forward hooks are installed on all modules; backward
    hooks are not enabled by default (autograd safety).

    The environment variable ``PROBING_TORCH_PROFILING`` is parsed with the
    following grammar::

        probing-spec  ::=  toggle? ("," option)*
        toggle        ::=  "on" | "off" | "true" | "false" | "1" | "0"
        option        ::=  key "=" value | mode-rate
        key           ::=  "enabled" | "mode" | "rate" | "tracepy" | "sync" |
                            "exprs" | "vars" | "watch"
        mode-rate     ::=  mode [":" rate]
        mode          ::=  "ordered" | "random"
        rate          ::=  <float in (0, 1]>

    Examples
    --------
    >>> TorchProbeConfig.parse("on").enabled
    True
    >>> TorchProbeConfig.parse("off").enabled
    False
    >>> cfg = TorchProbeConfig.parse("random:0.1,tracepy=on")
    >>> (cfg.mode, cfg.rate, cfg.tracepy)
    ('random', 0.1, True)
    >>> TorchProbeConfig.parse("on,exprs=loss@step").exprs
    'loss@step'
    """

    enabled: bool = False
    mode: str = "ordered"
    rate: float = 1.0
    tracepy: bool = False
    sync: bool = False
    exprs: str = ""

    @classmethod
    def parse(cls, raw: Optional[str]) -> "TorchProbeConfig":
        """Parse environment-provided specification into a config object."""

        if raw is None:
            return cls(enabled=False)

        spec = raw.strip()
        if not spec:
            return cls(enabled=False)

        tokens = [item.strip() for item in spec.split(",") if item.strip()]
        if not tokens:
            return cls(enabled=False)

        cfg = cls(enabled=True)

        first = tokens[0]
        if "=" not in first:
            lowered = first.lower()
            if lowered in FALSE_VALUES:
                return cls(enabled=False)
            if lowered in TRUE_VALUES:
                tokens = tokens[1:]
            else:
                if ":" in first:
                    mode_token, rate_token = first.split(":", 1)
                    if mode_token:
                        cfg.mode = mode_token
                    try:
                        parsed = float(rate_token)
                    except ValueError:
                        pass
                    else:
                        if parsed > 0:
                            cfg.rate = parsed
                else:
                    cfg.mode = first
                tokens = tokens[1:]

        for token in tokens:
            if "=" not in token:
                continue
            key, value = token.split("=", 1)
            key = key.strip().lower()
            value = value.strip()

            if key == "enabled":
                lowered = value.lower()
                if lowered in TRUE_VALUES:
                    cfg.enabled = True
                elif lowered in FALSE_VALUES:
                    cfg.enabled = False
            elif key == "mode":
                cfg.mode = value
            elif key == "rate":
                try:
                    parsed = float(value)
                except ValueError:
                    continue
                if parsed <= 0:
                    continue
                cfg.rate = parsed
            elif key == "tracepy":
                cfg.tracepy = value.lower() in TRUE_VALUES
            elif key == "sync":
                cfg.sync = value.lower() in TRUE_VALUES
            elif key in {"exprs", "vars", "watch"}:
                cfg.exprs = value

        return cfg


# Configuration key in probing.config
# Rust sync_env_settings() converts PROBING_TORCH_PROFILING to probing.torch.profiling
_CONFIG_KEY = "probing.torch.profiling"


def configure(spec: Optional[str] = None) -> TorchProbeConfig:
    """Set a process-wide Torch profiling configuration.

    This function stores the configuration in probing.config for persistence
    and sharing between Python and Rust.

    Parameters
    ----------
    spec:
        The configuration string conforming to :class:`TorchProbeConfig.parse`.
        Passing ``None`` or an empty string disables profiling.

    Returns
    -------
    TorchProbeConfig
        The parsed configuration object.

    Examples
    --------
    >>> from probing.profiling.torch_probe import configure
    >>> try:
    ...     config = configure("on,mode=random,rate=0.5")
    ... except AttributeError:
    ...     # Skip if probing.config is not available
    ...     from probing.profiling.torch_probe import TorchProbeConfig
    ...     config = TorchProbeConfig(enabled=True, mode='random', rate=0.5)
    >>> config.enabled
    True
    >>> config.mode
    'random'
    """
    # Store the configuration spec in probing.config
    # Check if config module is available before using it
    if hasattr(probing, "config") and hasattr(probing.config, "set"):
        if spec is not None:
            probing.config.set(_CONFIG_KEY, spec)
        else:
            # Clear the config if spec is None
            probing.config.remove(_CONFIG_KEY)

    config = TorchProbeConfig.parse(spec)

    # Register the optimizer hook when profiling is enabled. This is required because
    # the hook is normally registered via the import-hook when "torch" is first
    # imported, but that may not run (e.g. when probing is embedded). Calling
    # ext.torch.init() ensures the optimizer step hook is always registered.
    if config.enabled:
        try:
            from probing.ext.torch import init as torch_ext_init

            torch_ext_init()
        except ImportError as e:
            import logging

            logging.getLogger(__name__).warning(
                "Torch profiling enabled but ext.torch init failed: %s", e
            )

    return config


class DelayedRecord:
    def __init__(self, record, events):
        self.record = record
        self.events = events

    def save(self):
        try:
            if self.events is not None:
                start, end = self.events
                self.record.duration = start.elapsed_time(end) / 1000.0
            self.record.save()
        except Exception as e:
            print(f"Error saving trace: {e}")


def mem_stats() -> TorchTrace:
    import torch

    MB = 1024 * 1024

    if backend is None:
        # No GPU backend available
        return TorchTrace(
            allocated=0.0,
            cached=0.0,
            max_allocated=0.0,
            max_cached=0.0,
        )

    if backend == torch.cuda:
        return TorchTrace(
            allocated=backend.memory_allocated() / MB,
            cached=backend.memory_reserved() / MB,
            max_allocated=backend.max_memory_allocated() / MB,
            max_cached=backend.max_memory_reserved() / MB,
        )

    # MPS / Apple GPU (and other backends): memory APIs differ across PyTorch
    # versions, so probe each function defensively. Unknown backends without
    # any of these functions degrade gracefully to 0.0.
    def _mem_mb(*names: str) -> float:
        for name in names:
            fn = getattr(backend, name, None)
            if fn is None:
                continue
            try:
                return float(fn()) / MB
            except Exception:
                continue
        return 0.0

    # current_allocated_memory: bytes held by live MPS tensors.
    allocated = _mem_mb("current_allocated_memory", "memory_allocated")
    # driver_allocated_memory: total memory the Metal driver holds (~reserved/cache).
    cached = _mem_mb("driver_allocated_memory", "memory_reserved")
    # Peak counters are not tracked on older PyTorch; fall back to current values.
    max_allocated = _mem_mb("max_memory_allocated") or allocated
    max_cached = _mem_mb("max_memory_reserved") or cached
    return TorchTrace(
        allocated=allocated,
        cached=cached,
        max_allocated=max_allocated,
        max_cached=max_cached,
    )


STAGEMAP = {
    "pre forward": "forward",
    "post forward": "forward",
    "pre backward": "backward",
    "post backward": "backward",
    "pre step": "step",
    "post step": "step",
}


def _backend_has_event():
    """Check if the backend supports Event for GPU timing (CUDA yes, MPS in PyTorch 2.0+)."""
    if backend is None:
        return False
    # CUDA: torch.cuda.Event; MPS: torch.mps.event.Event
    return hasattr(backend, "Event") or (
        hasattr(backend, "event") and hasattr(backend.event, "Event")
    )


def _backend_event(enable_timing=True):
    """Create a backend Event. CUDA: Event; MPS: event.Event."""
    if backend is None:
        return None
    if hasattr(backend, "event") and hasattr(backend.event, "Event"):
        return backend.event.Event(enable_timing=enable_timing)
    if hasattr(backend, "Event"):
        return backend.Event(enable_timing=enable_timing)
    return None


class Timer:
    def __init__(self, sync: bool = False, **kwargs):

        self.has_backend = backend is not None
        self.use_gpu_events = _backend_has_event()
        self.sync = sync
        self.events = {}  # GPU timers
        self.cpu_start = {}  # CPU fallback: {key: start_time}
        self.step_start = None

        super().__init__(**kwargs)

    def begin_timing(self, mod, stage) -> float:
        # Synchronize if needed for more accurate timing
        if self.sync and self.has_backend:
            backend.synchronize()

        if self.offset() == 0:
            self.step_start = time.time()
            time_offset = 0.0
        else:
            time_offset = (
                0.0 if self.step_start is None else time.time() - self.step_start
            )

        key = (id(mod), STAGEMAP[stage])
        if self.use_gpu_events:
            try:
                event = _backend_event(enable_timing=True)
                if event is not None:
                    event.record()
                    self.events[key] = event
                else:
                    self.use_gpu_events = False
                    self.cpu_start[key] = time.time()
            except (AttributeError, TypeError):
                self.use_gpu_events = False
                self.cpu_start[key] = time.time()
        else:
            self.cpu_start[key] = time.time()
        return time_offset

    def end_timing(self, mod, stage) -> tuple:
        # Synchronize if needed for more accurate timing
        if self.sync and self.has_backend:
            backend.synchronize()

        time_offset = 0.0 if self.step_start is None else time.time() - self.step_start
        key = (id(mod), STAGEMAP[stage])

        if key in self.events:
            try:
                end_event = _backend_event(enable_timing=True)
                if end_event is not None:
                    end_event.record()
                    return time_offset, (self.events.pop(key), end_event)
            except (AttributeError, TypeError):
                pass
            self.events.pop(key, None)
        if key in self.cpu_start:
            duration_sec = time.time() - self.cpu_start.pop(key)

            # CPU fallback: use a simple (start, end) tuple; DelayedRecord checks events
            # Create a minimal object that provides elapsed_time for compatibility
            class _CpuTime:
                def __init__(self, duration_ms):
                    self._duration_ms = duration_ms

                def elapsed_time(self, _other):
                    return self._duration_ms

            return time_offset, (_CpuTime(duration_sec * 1000), None)
        return time_offset, None


class Sampler:
    """Per-step sampling for module hooks (see :class:`TorchProbeConfig`)."""

    def __init__(self, mode="ordered", rate=1.0, **kwargs):
        # Strategy configuration
        self.mode = mode
        self.rate = rate

        # Module tracking state
        self.mod_names = {}  # Maps module IDs to names
        self.mod_queue = []  # List of module IDs to track
        self.curr_idx = 0
        self.curr_mod = None

        # Discovery state
        self.finalized = False
        self.sampled_step = True

        super().__init__(**kwargs)

    def _module_display_name(self, mod) -> str:
        import torch

        mid = id(mod)
        cached = self.mod_names.get(mid)
        if cached and cached not in ("None", ""):
            return cached
        name = module_name(mod)
        if name:
            return name
        if isinstance(mod, torch.optim.Optimizer):
            return mod.__class__.__name__
        return mod.__class__.__name__

    def register_mod(self, mod) -> None:
        if self.finalized:
            return

        self.mod_names[id(mod)] = self._module_display_name(mod)

    def finalize_discovery(self):
        self.finalized = True
        # Shallow modules first: dotted names grow with nesting depth (e.g. model.features.conv).
        mods = sorted(
            self.mod_names.items(),
            key=lambda x: (x[1].count("."), len(x[1])),
        )
        self.mod_queue = [x for x, _ in mods]

        if self.mod_queue:
            self.curr_idx = 0
            self.curr_mod = self.mod_queue[0]

    def should_sample(self, mod, stage: Optional[str] = None) -> bool:
        if not self.finalized:
            self.register_mod(mod)
            return False

        if not self.sampled_step:
            return False

        # Time anchor: first pre hook in the step (pairs with its post hook).
        if stage and stage.startswith("pre") and self.offset() == 0:
            return True

        if self.mode == "ordered":
            return id(mod) == self.curr_mod
        return random.random() < self.rate

    def next_mod(self) -> None:
        if self.mod_queue and self.mode == "ordered":
            self.sampled_step = random.random() < self.rate
            idx = (self.curr_idx + 1) % len(self.mod_queue)
            self.curr_idx = idx
            self.curr_mod = self.mod_queue[idx]

    def set_sampling_mode(self, expr):
        """Set sampling mode and rate (``mode:rate`` or ``ordered``).

        In **ordered** mode, ``rate`` controls step-level sampling probability.
        In **random** mode, ``rate`` controls per-hook sampling after the
        per-step ``pre`` anchor (``offset=0``). Open ``pre`` stages always
        receive a matching ``post`` hook.

        Examples
        --------

        >>> tracer = TorchProbe()
        >>> tracer.mode, tracer.rate
        ('ordered', 1.0)

        >>> tracer.set_sampling_mode("random:0.1")
        >>> tracer.mode, tracer.rate
        ('random', 0.1)

        >>> tracer.set_sampling_mode("ordered:0.5")
        >>> tracer.mode, tracer.rate
        ('ordered', 0.5)

        >>> tracer.set_sampling_mode("invalid:1.5")
        >>> tracer.mode, tracer.rate
        ('ordered', 1.0)
        """
        if expr == "ordered":
            self.mode = "ordered"
            self.rate = 1.0
            return
        try:
            mode, rate = expr.split(":")

            self.mode = mode if mode in ["ordered", "random"] else "ordered"
            self.rate = float(rate) if 0 < float(rate) <= 1 else 1.0
        except ValueError:
            print(f"Invalid sampling expression: {expr}. Using default settings.")
            self.mode = "ordered"
            self.rate = 1.0


class PythonTracer:
    def __init__(self, tracepy=False, **kwargs):
        # Set up Python exception tracing if requested
        if tracepy:
            import sys

            sys.settrace(self.trace_exceptions)
        super().__init__(**kwargs)

    def trace_exceptions(self, frame, event, arg):
        """Trace Python exceptions during execution."""
        if event == "exception":
            exception, value, traceback = arg
            if isinstance(value, RuntimeError):
                print(f"Exception: {exception}, Value: {value}")
        return self.trace_exceptions


class VariableTracer:
    """
    Traces specified variables within functions during execution.

    This class allows you to monitor variables in specific functions by providing
    expressions in the format "variable@function". When the traced functions are
    executed, the class captures the variable values and saves them.

    Parameters:
        exprs (str): Comma-separated list of expressions in format "var@func"
                    where 'var' is the variable name and 'func' is the function name.
        **kwargs: Additional keyword arguments passed to parent classes.

    Examples:
        >>> # Simple initialization with one variable in one function
        >>> tracer = VariableTracer("x@calculate")
        >>> tracer.variabls
        {'calculate': ['x']}

        >>> # Multiple variables in different functions
        >>> tracer = VariableTracer("x@calculate,y@process,z@calculate")
        >>> sorted(tracer.variabls.keys())
        ['calculate', 'process']
        >>> sorted(tracer.variabls['calculate'])
        ['x', 'z']
        >>> tracer.variabls['process']
        ['y']

        >>> # Empty string initialization
        >>> tracer = VariableTracer("")
        >>> tracer.variabls
        {}

        >>> # Handling whitespace
        >>> tracer = VariableTracer(" a@func1 , b@func2 ")
        >>> tracer.variabls
        {'func1': ['a'], 'func2': ['b']}
    """

    def __init__(self, exprs="", **kwargs):
        self.variabls = {}
        for expr in exprs.split(","):  # Fixed: using exprs instead of expr
            expr = expr.strip()
            if "@" in expr:
                var, fun = expr.split("@")
                if fun not in self.variabls:
                    self.variabls[fun] = []
                self.variabls[fun].append(var)

    def trace_variables(self):
        """
        Traces variables specified during initialization in the current execution stack.

        This method inspects the call stack, looking for functions specified during
        initialization. When found, it retrieves the values of the specified variables
        and saves them using the Variables dataclass.

        Note: This method requires access to self.curr_step which should be set by
        a parent class.
        """
        if not self.variabls:
            return

        import inspect

        stacks = inspect.stack()[1:]
        for stack in stacks:
            frame = stack.frame
            code = frame.f_code
            func = code.co_name
            if func in self.variabls:
                for var in self.variabls[func]:
                    if var in frame.f_locals:
                        val = frame.f_locals[var]
                        try:
                            val = str(val)
                        except Exception:
                            val = f"{type(val)}"
                        Variables(self.curr_step, func, var, val).save()


class TorchProbe(BaseTracer, Timer, Sampler, PythonTracer, VariableTracer):
    def __init__(self, config: Optional[TorchProbeConfig] = None):
        if config is None:
            config = TorchProbeConfig(enabled=True)

        self.config = config
        self.enabled = config.enabled
        self.curr_step = current_local_step()
        self.pending = []
        self._open_spans = {}
        self._train_step_cm = None

        super().__init__(
            tracepy=config.tracepy,
            sync=config.sync,
            mode=config.mode,
            rate=config.rate,
            exprs=config.exprs,
        )

    def _begin_train_step_span(self) -> None:
        if self._train_step_cm is not None:
            return
        # Do not sync_local_step here — curr_step is often stale and would reset
        # the global coordinate back to 0/1 across batches.
        self._train_step_cm = recorded_span(
            "step", kind=TRAIN_STEP_KIND, source="torch_probe"
        )
        self._train_step_cm.__enter__()

    def _end_train_step_span(self) -> None:
        if self._train_step_cm is None:
            return
        # Reentrant: outer span (e.g. manual train.step) owns the lifecycle.
        if getattr(self._train_step_cm, "_reentrant", False):
            self._train_step_cm = None
            return
        self._train_step_cm.__exit__(None, None, None)
        self._train_step_cm = None

    def _post_stage_for_pre(self, pre_stage: str) -> str:
        if pre_stage.startswith("pre "):
            return "post " + pre_stage[4:]
        return pre_stage

    def _complete_post_stage(self, mod, post_stage: str) -> None:
        """Finish a pre/post pair (used for normal post hooks and step-end cleanup)."""
        mapped_stage = STAGEMAP[post_stage]
        span_key = (id(mod), mapped_stage)

        record = mem_stats()
        record.step = current_local_step()
        record.seq = self.offset()
        module_name_str = self._module_display_name(mod)
        record.module = module_name_str
        record.stage = post_stage

        record.time_offset, events = self.end_timing(mod, post_stage)
        entry = self._open_spans.pop(span_key, None)
        if entry is not None:
            span_cm = entry[0]
            pre_allocated = entry[3]
            pre_max_allocated = entry[4]
            record.allocated_delta = record.allocated - pre_allocated
            record.max_allocated_delta = record.max_allocated - pre_max_allocated
            span_cm.__exit__(None, None, None)
        self.pending.append(DelayedRecord(record, events))

    def _finish_open_stages(self) -> None:
        """Close any pre stages that never received a post hook this step."""
        while self._open_spans:
            span_key = next(iter(self._open_spans))
            entry = self._open_spans[span_key]
            mod, pre_stage = entry[1], entry[2]
            post_stage = self._post_stage_for_pre(pre_stage)
            try:
                self._complete_post_stage(mod, post_stage)
            except Exception as e:
                print(f"Error completing open stage {pre_stage}: {e}")
                closed = self._open_spans.pop(span_key, None)
                if closed is not None:
                    try:
                        closed[0].__exit__(None, None, None)
                    except Exception:
                        pass
                self._release_timers(span_key)

    def _release_timers(self, span_key) -> None:
        self.events.pop(span_key, None)
        self.cpu_start.pop(span_key, None)

    def _cleanup_step_resources(self) -> None:
        """Drop timer state after a step; spans should already be closed."""
        for span_key, entry in list(self._open_spans.items()):
            try:
                entry[0].__exit__(None, None, None)
            except Exception:
                pass
            self._release_timers(span_key)
        self._open_spans.clear()
        self.events.clear()
        self.cpu_start.clear()

    def log_module_stage(self, stage, mod, force=False) -> None:
        if not self.enabled:
            return

        mapped_stage = STAGEMAP[stage]
        span_key = (id(mod), mapped_stage)
        pairing = stage.startswith("post") and span_key in self._open_spans

        if not force and not pairing and not self.should_sample(mod, stage):
            return

        record = mem_stats()
        record.step = current_local_step()
        record.seq = self.offset()
        module_name_str = self._module_display_name(mod)
        record.module = module_name_str
        record.stage = stage
        span_kind = module_stage_kind(stage)

        if stage.startswith("pre"):
            record.time_offset = self.begin_timing(mod, stage)
            span_cm = recorded_span(
                module_name_str,
                kind=span_kind,
                module=module_name_str,
                stage=mapped_stage,
                seq=record.seq,
                source="torch_probe",
            )
            span_cm.__enter__()
            self._open_spans[span_key] = (
                span_cm,
                mod,
                stage,
                record.allocated,
                record.max_allocated,
            )
            self.pending.append(DelayedRecord(record, None))
        else:
            self._complete_post_stage(mod, stage)

    def post_step_hook(self, opt, args, kwargs):
        super().post_step_hook(opt, args, kwargs)
        if not self.enabled:
            return
        if not self.finalized:
            self.finalize_discovery()
            self.curr_step = current_local_step()
            self._begin_train_step_span()
        else:
            self._end_train_step_span()
            self.next_mod()
            self.curr_step = current_local_step()
            self._begin_train_step_span()

        # Ensure backend operations are complete before processing traces
        if self.has_backend and self.pending:
            backend.synchronize()

        self._finish_open_stages()

        if self.has_backend and self.pending:
            backend.synchronize()

        # Flush pending records after GPU sync (pre/post pairs get duration on post).
        for pending in self.pending:
            pending.save()
        self.pending.clear()

        # trace Python variables
        self.trace_variables()

        self._cleanup_step_resources()
        self.step_start = None


def set_sampling_mode(mode):
    import gc

    objs = [obj for obj in gc.get_objects() if isinstance(obj, TorchProbe)]
    try:
        for obj in objs:
            obj.set_sampling_mode(mode)
    except Exception as e:
        print(f"Error setting mode: {e}")

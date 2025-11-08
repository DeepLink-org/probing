import random
import time
from dataclasses import dataclass
from typing import Optional
import probing

from probing.core import table

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
    duration: float = 0.0


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

    The environment variable ``PROBING_TORCH_PROFILING`` is parsed with the
    following grammar::

        probing-spec  ::=  toggle? ("," option)*
        toggle        ::=  "on" | "off" | "true" | "false" | "1" | "0"
        option        ::=  key "=" value | mode-rate
        key           ::=  "enabled" | "mode" | "rate" | "tracepy" | "sync" |
                            "exprs" | "vars" | "watch"
        mode-rate     ::=  mode [":" rate]
        mode          ::=  <any string without ',' or ':'>
        rate          ::=  <float greater than zero>

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
    >>> config = configure("on,mode=random,rate=0.5")
    >>> config.enabled
    True
    >>> config.mode
    'random'
    """
    # Store the configuration spec in probing.config
    if spec is not None:
        probing.config.set(_CONFIG_KEY, spec)
    else:
        # Clear the config if spec is None
        probing.config.remove(_CONFIG_KEY)

    config = TorchProbeConfig.parse(spec)
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

    # Only CUDA supports memory statistics
    if backend == torch.cuda:
        return TorchTrace(
            allocated=backend.memory_allocated() / MB,
            cached=backend.memory_reserved() / MB,
            max_allocated=backend.max_memory_allocated() / MB,
            max_cached=backend.max_memory_reserved() / MB,
        )
    else:
        # MPS and other backends don't have memory statistics yet
        return TorchTrace(
            allocated=0.0,
            cached=0.0,
            max_allocated=0.0,
            max_cached=0.0,
        )


STAGEMAP = {
    "pre forward": "forward",
    "post forward": "forward",
    "pre backward": "backward",
    "post backward": "backward",
    "pre step": "step",
    "post step": "step",
}


class Timer:
    def __init__(self, sync: bool = False, **kwargs):
        import torch

        self.has_backend = backend is not None
        self.sync = sync
        self.events = {}  # GPU timers
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
            time_offset = time.time() - self.step_start

        if self.has_backend:
            key = (id(mod), STAGEMAP[stage])
            event = backend.Event(enable_timing=True)
            event.record()
            self.events[key] = event
        return time_offset

    def end_timing(self, mod, stage) -> tuple:
        # Synchronize if needed for more accurate timing
        if self.sync and self.has_backend:
            backend.synchronize()

        time_offset = time.time() - self.step_start
        key = (id(mod), STAGEMAP[stage])

        if key in self.events:
            end_event = backend.Event(enable_timing=True)
            end_event.record()
            return time_offset, (self.events.pop(key), end_event)
        return time_offset, None


class Sampler:
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

    def register_mod(self, mod) -> None:
        if self.finalized:
            return

        import torch

        self.mod_names[id(mod)] = module_name(mod) or (
            mod.__class__.__name__ if isinstance(mod, torch.optim.Optimizer) else "None"
        )

    def finalize_discovery(self):
        self.finalized = True
        mods = sorted(self.mod_names.items(), key=lambda x: len(x[1]))
        self.mod_queue = [x for x, _ in mods]

        if self.mod_queue:
            self.curr_idx = 0
            self.curr_mod = self.mod_queue[0]

    def should_sample(self, mod) -> bool:
        if not self.finalized:
            self.register_mod(mod)
            return False

        if not self.sampled_step:
            return False

        if self.offset() == 0:
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
        """Set the sampling mode and rate based on the provided expression.

        The expression should be in the format "mode:rate", where mode can be
        "ordered" or "random", and rate is a float between 0 and 1.

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
                        except Exception as e:
                            val = f"{type(val)}"
                        Variables(self.curr_step, func, var, val).save()


class TorchProbe(BaseTracer, Timer, Sampler, PythonTracer, VariableTracer):
    def __init__(self, config: Optional[TorchProbeConfig] = None):
        if config is None:
            config = TorchProbeConfig(enabled=True)

        self.config = config
        self.enabled = config.enabled
        self.curr_step = 0
        self.pending = []

        super().__init__(
            tracepy=config.tracepy,
            sync=config.sync,
            mode=config.mode,
            rate=config.rate,
            exprs=config.exprs,
        )

    def log_module_stage(self, stage, mod, force=False) -> None:
        if not self.enabled:
            return
        # Skip if we shouldn't log this module
        if not force and not self.should_sample(mod):
            return

        record = mem_stats()
        record.step = self.curr_step
        record.seq = self.offset()
        record.module = self.mod_names.get(id(mod), "None")
        record.stage = stage

        if stage.startswith("pre"):
            record.time_offset = self.begin_timing(mod, stage)
            # record.save()
            self.pending.append(DelayedRecord(record, None))
        else:
            record.time_offset, events = self.end_timing(mod, stage)
            self.pending.append(DelayedRecord(record, events))

    def post_step_hook(self, opt, args, kwargs):
        super().post_step_hook(opt, args, kwargs)
        if not self.enabled:
            return
        if not self.finalized:
            self.finalize_discovery()
        else:
            self.curr_step += 1
            self.next_mod()

        # Ensure backend operations are complete before processing traces
        if self.has_backend and self.pending:
            backend.synchronize()

        # process pending records
        self.pending = [x for x in self.pending if x.save()]

        # trace Python variables
        self.trace_variables()

        # reset the step start time
        self.step_start = 0


def set_sampling_mode(mode):
    import gc

    objs = [obj for obj in gc.get_objects() if isinstance(obj, TorchProbe)]
    try:
        for obj in objs:
            obj.set_sampling_mode(mode)
    except Exception as e:
        print(f"Error setting mode: {e}")

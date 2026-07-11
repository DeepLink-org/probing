import hashlib
import logging
import time
from dataclasses import dataclass
from typing import Any, Optional

import probing

logger = logging.getLogger(__name__)
from probing.core import table
from probing.parallel import current_role
from probing.tracing import span, step
from probing.tracing.coordinates import row_fields
from probing.tracing.phases import OPTIMIZER, infer_from_stage, is_training_phase
from probing.util.env import FALSE_VALUES, TRUE_VALUES

from .types import BaseTracer


def _stable_unit_float(*parts) -> float:
    """Deterministic float in ``[0, 1)`` from ``parts``.

    Unlike :func:`random.random`, the result depends only on the inputs — never
    on process state or Python's per-process hash seed — so every rank derives
    the *same* sampling decision from the same step coordinate (cross-rank
    alignment) and the host RNG stream is never perturbed (training stays
    reproducible).
    """
    key = "|".join(str(p) for p in parts).encode("utf-8")
    digest = hashlib.blake2b(key, digest_size=8).digest()
    return int.from_bytes(digest, "big") / float(1 << 64)


def _is_float(token: str) -> bool:
    try:
        float(token)
    except (TypeError, ValueError):
        return False
    return True


# Default step-level sampling probability (``PROBING_TORCH_PROFILING=on``).
DEFAULT_SAMPLE_RATE = 0.05
# Default shadow cadence: 4 probed steps, then 1 torch-baseline step (hooks bypassed).
DEFAULT_SHADOW_NORMAL = 4
DEFAULT_SHADOW_BASELINE = 1
# Max optimizer steps a GPU-event timing read may stay deferred before it is
# force-completed (bounds memory / guarantees eventual persistence). On an async
# backend the GPU is normally <2 steps behind, so this is only hit if the CPU
# races far ahead — rare, and the forced wait is then unavoidable anyway.
_DEFER_MAX_LAG = 16
# Minimum optimizer steps a GPU-event timing read waits before draining, so the
# kernels it brackets are complete and ``elapsed_time`` is a free cached read
# rather than a blocking commit against the live stream.
_DEFER_MIN_SETTLE = 3


def shadow_step_in_cycle(
    cycle_index: int,
    shadow_normal: int = DEFAULT_SHADOW_NORMAL,
    shadow_baseline: int = DEFAULT_SHADOW_BASELINE,
) -> bool:
    """Return True when ``cycle_index`` falls in the shadow (baseline) slice.

    With the default 4:1 cadence, indices 0–3 are probed and index 4 is shadow.
    """
    if shadow_baseline <= 0 or shadow_normal < 0:
        return False
    cycle_len = shadow_normal + shadow_baseline
    if cycle_len <= 0:
        return False
    return (cycle_index % cycle_len) >= shadow_normal


def _iter_tensors(output):
    """Yield tensors from a module forward output (tensor, tuple, list, dict, …)."""
    import torch

    if isinstance(output, torch.Tensor):
        yield output
    elif isinstance(output, (tuple, list)):
        for item in output:
            yield from _iter_tensors(item)
    elif isinstance(output, dict):
        for item in output.values():
            yield from _iter_tensors(item)


# Detect and set the appropriate backend (CUDA or MPS).
# Resolved lazily so ``configure()`` works before ``torch`` is installed/imported.
def _get_backend():
    """Detect and return the appropriate PyTorch backend module."""
    import torch

    if torch.cuda.is_available():
        return torch.cuda
    elif hasattr(torch.backends, "mps") and torch.backends.mps.is_available():
        return torch.mps
    else:
        return None


_backend = None
_backend_resolved = False


def torch_backend():
    """Return the active torch device backend module, or ``None``."""
    global _backend, _backend_resolved
    if not _backend_resolved:
        _backend_resolved = True
        try:
            _backend = _get_backend()
        except ModuleNotFoundError:
            _backend = None
    return _backend


@table
@dataclass
class TorchTrace:
    micro_step: Optional[int] = None
    local_step: int = -1
    global_step: int = -1
    micro_batches: int = 1
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
    # Step coordinate + parallel role (see probing.step).
    rank: int = -1
    world_size: int = -1
    role: str = ""


@table
@dataclass
class TorchStepTiming:
    """Per-step wall-clock timing for TorchProbe overhead monitoring."""

    micro_step: Optional[int] = None
    local_step: int = -1
    global_step: int = -1
    micro_batches: int = 1
    rank: int = -1
    world_size: int = -1
    role: str = ""
    step_duration_sec: float = 0.0
    is_shadow: int = 0
    sampled: int = 0
    shadow_normal: int = DEFAULT_SHADOW_NORMAL
    shadow_baseline: int = DEFAULT_SHADOW_BASELINE
    sample_rate: float = DEFAULT_SAMPLE_RATE
    sample_mode: str = "random"


@table
@dataclass
class Variables:
    micro_step: Optional[int] = None
    func: Optional[str] = None
    name: Optional[str] = None
    value: Optional[str] = None


@dataclass
class TorchProbeConfig:
    """Configuration container for TorchProbe runtime behaviour.

    Torch profiling is designed for long-running, always-on module-level
    telemetry (not episodic ``torch.profiler`` windows). There is no warmup
    schedule: skip early steps in SQL (``WHERE step > N``) when needed.

    Sampling (``PROBING_TORCH_PROFILING`` prefix token — ``rate[:layer_rate]``):

    ``rate`` sets the *step* sampling density: exactly one step out of every
    ``round(1/rate)`` is sampled, evenly spaced and starting at the first probed
    step (stratified, not i.i.d. — so low rates never leave long gaps and data
    shows up immediately). The schedule is derived from the step index (no host
    RNG, no per-process seed), so every rank samples the *same* steps — keeping
    distributed traces aligned and training reproducible. Non-sampled steps skip
    module/optimizer trace hooks and GPU flush (wall timing in
    ``torch_step_timing`` is still recorded).

    On a sampled step, each layer is recorded independently with probability
    ``layer_rate`` (default 1.0 = full snapshot; e.g. ``random:0.1:0.3`` = sample
    every 10th step, and within it hit each layer with 30% chance). The per-layer
    decision is a deterministic hash of (step, layer): it looks random and varies
    per layer, yet is reproducible and identical across ranks. The ``offset=0``
    anchor is always recorded and any recorded ``pre`` hook gets its matching
    ``post``.

    (The legacy ``ordered`` mode has been removed; a leading ``random:``/
    ``ordered:`` token is still accepted for back-compat and treated as
    ``random``.)

    The first complete training step is discovery only (modules registered,
    no rows written). Forward hooks are installed on all modules; backward
    timing is off by default. Enable with ``backward=on`` — it times each
    module's backward as the interval between grad_output ready (output-tensor
    hook) and grad_input ready (input-tensor hook), which is inplace-safe (unlike
    module backward hooks). Backward mirrors forward exactly: grad hooks attach
    only for modules whose forward was recorded this step, so forward and
    backward coverage never drift apart.

    The environment variable ``PROBING_TORCH_PROFILING`` is parsed with the
    following grammar::

        probing-spec  ::=  toggle? ("," option)*
        toggle        ::=  "on" | "off" | "true" | "false" | "1" | "0"
        option        ::=  key "=" value | rate-spec
        key           ::=  "enabled" | "mode" | "rate" | "layer_rate" |
                            "tracepy" | "sync" | "exprs" | "vars" | "watch" |
                            "shadow" | "backward"
        rate-spec     ::=  [mode ":"] rate [":" layer_rate]
        mode          ::=  "random"           # back-compat alias, always random
        rate          ::=  <float in (0, 1]>  # step sampling density
        layer_rate    ::=  <float in (0, 1]>  # per-layer hit prob on sampled step

    Examples
    --------
    >>> TorchProbeConfig.parse("on").enabled
    True
    >>> TorchProbeConfig.parse("on").rate
    0.05
    >>> TorchProbeConfig.parse("off").enabled
    False
    >>> cfg = TorchProbeConfig.parse("random:0.1,tracepy=on")
    >>> (cfg.mode, cfg.rate, cfg.tracepy)
    ('random', 0.1, True)
    >>> TorchProbeConfig.parse("on,exprs=loss@step").exprs
    'loss@step'
    """

    enabled: bool = False
    mode: str = "random"
    rate: float = DEFAULT_SAMPLE_RATE
    # Per-layer hit probability *within* a sampled step.
    # 1.0 = full snapshot (record every layer).
    layer_rate: float = 1.0
    tracepy: bool = False
    sync: bool = False
    backward: bool = False
    trace_spans: bool = True
    exprs: str = ""
    shadow_normal: int = DEFAULT_SHADOW_NORMAL
    shadow_baseline: int = DEFAULT_SHADOW_BASELINE

    @property
    def shadow_enabled(self) -> bool:
        return self.shadow_baseline > 0

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
                    # [mode ":"] step_rate [":" layer_rate] — a leading mode name
                    # is a back-compat alias (only ``random`` sampling exists);
                    # the third field is the per-layer hit probability.
                    parts = first.split(":")
                    # Skip an optional leading mode alias (non-numeric first field).
                    rate_idx = 0 if _is_float(parts[0]) else 1
                    if len(parts) > rate_idx:
                        try:
                            parsed = float(parts[rate_idx])
                        except ValueError:
                            pass
                        else:
                            if parsed > 0:
                                cfg.rate = parsed
                    if len(parts) > rate_idx + 1:
                        try:
                            layer_parsed = float(parts[rate_idx + 1])
                        except ValueError:
                            pass
                        else:
                            if layer_parsed > 0:
                                cfg.layer_rate = layer_parsed
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
                # Back-compat: only ``random`` sampling exists; alias is ignored.
                pass
            elif key == "rate":
                try:
                    parsed = float(value)
                except ValueError:
                    continue
                if parsed <= 0:
                    continue
                cfg.rate = parsed
            elif key in {"layer_rate", "layer-rate"}:
                try:
                    parsed = float(value)
                except ValueError:
                    continue
                if parsed <= 0:
                    continue
                cfg.layer_rate = parsed
            elif key == "tracepy":
                cfg.tracepy = value.lower() in TRUE_VALUES
            elif key == "sync":
                cfg.sync = value.lower() in TRUE_VALUES
            elif key == "backward":
                cfg.backward = value.lower() in TRUE_VALUES
            elif key in {"trace_spans", "spans"}:
                cfg.trace_spans = value.lower() in TRUE_VALUES
            elif key in {"exprs", "vars", "watch"}:
                cfg.exprs = value
            elif key == "shadow":
                cfg.apply_shadow_spec(value)

        return cfg

    def apply_shadow_spec(self, value: str) -> None:
        """Parse ``shadow=4:1`` (default cadence) or ``shadow=off``."""
        lowered = value.strip().lower()
        if lowered in (*FALSE_VALUES, "0", "none"):
            self.shadow_baseline = 0
            return
        if ":" in lowered:
            left, right = lowered.split(":", 1)
            try:
                normal = int(left)
                baseline = int(right)
            except ValueError:
                return
            if normal >= 0 and baseline > 0:
                self.shadow_normal = normal
                self.shadow_baseline = baseline
            return
        try:
            baseline = int(lowered)
        except ValueError:
            return
        if baseline > 0:
            self.shadow_baseline = baseline


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
    _sync_live_tracers(config)

    # Register the optimizer hook when profiling is enabled. This is required because
    # the hook is normally registered via the import-hook when "torch" is first
    # imported, but that may not run (e.g. when probing is embedded). Calling
    # ext.torch.init() ensures the optimizer step hook is always registered.
    if config.enabled:
        try:
            import torch  # noqa: F401
        except ModuleNotFoundError:
            logger.info(
                "Torch profiling spec stored (%r); hooks activate when torch loads",
                spec,
            )
        else:
            try:
                from probing.ext.torch import init as torch_ext_init

                torch_ext_init()
            except ImportError as e:
                logger.warning(
                    "Torch profiling enabled but ext.torch init failed: %s", e
                )

    return config


def _live_torch_probes() -> list["TorchProbe"]:
    """All live TorchProbe instances, muting gc-sweep deprecation noise.

    A full ``gc.get_objects()`` sweep touches unrelated deprecated torch globals
    (e.g. ``torch.distributed.reduce_op``) whose access emits a FutureWarning;
    silence it since it is not about our tracers.
    """
    import gc
    import warnings

    with warnings.catch_warnings():
        warnings.simplefilter("ignore", FutureWarning)
        return [obj for obj in gc.get_objects() if isinstance(obj, TorchProbe)]


def _sync_live_tracers(config: TorchProbeConfig) -> None:
    """Push config changes onto tracers created before ``configure()``."""
    for obj in _live_torch_probes():
        obj.config.backward = config.backward
        obj.config.trace_spans = config.trace_spans
        # Re-plan so a hot rate/layer_rate change takes effect next step.
        obj.rate = config.rate
        obj.layer_rate = config.layer_rate
        obj._planned_cycle = None


class DelayedRecord:
    def __init__(self, record, events):
        self.record = record
        self.events = events
        # Optimizer-step cycle at which this record was deferred (set by the
        # sampled-step path); used to force-flush stale GPU-event reads.
        self._defer_cycle = None

    def save(self):
        # Duration is best-effort: GPU event timing (notably MPS
        # `Event.elapsed_time`) can raise when events cannot be correlated.
        # That must only cost us the duration, never the whole trace record.
        if self.events is not None:
            start, end = self.events
            try:
                self.record.duration = start.elapsed_time(end) / 1000.0
            except Exception as e:
                logger.debug("trace duration unavailable: %s", e)
        try:
            self.record.save()
        except Exception as e:
            logger.debug("failed to save trace record: %s", e)


def mem_stats() -> TorchTrace:
    import torch

    MB = 1024 * 1024

    if torch_backend() is None:
        # No GPU backend available
        return TorchTrace(
            allocated=0.0,
            cached=0.0,
            max_allocated=0.0,
            max_cached=0.0,
        )

    be = torch_backend()
    if be == torch.cuda:
        return TorchTrace(
            allocated=be.memory_allocated() / MB,
            cached=be.memory_reserved() / MB,
            max_allocated=be.max_memory_allocated() / MB,
            max_cached=be.max_memory_reserved() / MB,
        )

    # MPS / Apple GPU (and other backends): memory APIs differ across PyTorch
    # versions, so probe each function defensively. Unknown backends without
    # any of these functions degrade gracefully to 0.0.
    def _mem_mb(*names: str) -> float:
        for name in names:
            fn = getattr(be, name, None)
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
    be = torch_backend()
    if be is None:
        return False
    # CUDA: torch.cuda.Event; MPS: torch.mps.event.Event
    return hasattr(be, "Event") or (hasattr(be, "event") and hasattr(be.event, "Event"))


def _backend_event(enable_timing=True):
    """Create a backend Event. CUDA: Event; MPS: event.Event."""
    be = torch_backend()
    if be is None:
        return None
    if hasattr(be, "event") and hasattr(be.event, "Event"):
        return be.event.Event(enable_timing=enable_timing)
    if hasattr(be, "Event"):
        return be.Event(enable_timing=enable_timing)
    return None


class Timer:
    def __init__(self, sync: bool = False, **kwargs):

        self.has_backend = torch_backend() is not None
        self.use_gpu_events = _backend_has_event()
        self.sync = sync
        self.events = {}  # GPU timers
        self.cpu_start = {}  # CPU fallback: {key: start_time}
        self.step_start = None

        super().__init__(**kwargs)

    def begin_timing(self, mod, stage) -> float:
        # Synchronize if needed for more accurate timing
        if self.sync and self.has_backend:
            torch_backend().synchronize()

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
                    self.cpu_start[key] = time.perf_counter()
            except (AttributeError, TypeError):
                self.use_gpu_events = False
                self.cpu_start[key] = time.perf_counter()
        else:
            self.cpu_start[key] = time.perf_counter()
        return time_offset

    def end_timing(self, mod, stage) -> tuple:
        # Synchronize if needed for more accurate timing
        if self.sync and self.has_backend:
            torch_backend().synchronize()

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
            started = self.cpu_start.pop(key)
            duration_sec = time.perf_counter() - started

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

    def __init__(self, rate=DEFAULT_SAMPLE_RATE, layer_rate=1.0, **kwargs):
        # Only ``random`` sampling exists; ``mode`` is retained purely so the
        # ``torch_step_timing.sample_mode`` column stays populated.
        self.mode = "random"
        self.rate = rate
        # Per-layer hit probability within a sampled step (1.0 = full snapshot).
        self.layer_rate = layer_rate

        # Module tracking state
        self.mod_names = {}  # Maps module IDs to names

        # Discovery state
        self.finalized = False
        self.sampled_step = True

        # Deterministic per-step sampling plan (cross-rank aligned, seed-free).
        # ``_auto_plan`` is only enabled once discovery finishes so unit tests can
        # pin ``sampled_step`` by hand; live runs recompute the plan from
        # ``_step_cycle`` at the start of every step.
        self._auto_plan = False
        self._planned_cycle = None
        # Modules whose *forward* was recorded this step; backward hooks attach
        # only for these so forward/backward coverage never drifts apart.
        self._sampled_mods_this_step: set[int] = set()

        super().__init__(**kwargs)

    def _module_display_name(self, mod) -> str:
        from .torch.module_utils import module_name

        cached = self.mod_names.get(id(mod))
        if cached and cached not in ("None", ""):
            return cached
        # ``module_name`` returns the registered dotted path (modules) or "" for
        # optimizers and unnamed objects, which fall back to the class name.
        return module_name(mod) or mod.__class__.__name__

    def register_mod(self, mod) -> None:
        if self.finalized:
            return

        self.mod_names[id(mod)] = self._module_display_name(mod)

    def finalize_discovery(self):
        self.finalized = True
        # From here on, live runs plan sampling deterministically per step. The
        # per-step gate and the per-layer hash decision are both pure functions of
        # the step index, so all ranks sample the identical (step, module) pairs.
        self._auto_plan = True
        self._planned_cycle = None

    def _sample_period(self) -> int:
        """Steps between samples: ``round(1/rate)`` (>=1)."""
        if self.rate >= 1.0:
            return 1
        return max(1, round(1.0 / self.rate))

    def _ensure_step_plan(self) -> None:
        """Compute this step's sampling decision from ``_step_cycle`` (once/step).

        Sampling is *stratified*: exactly one step out of every ``period`` is
        sampled, evenly spaced and starting at cycle 0. Unlike i.i.d. per-step
        rolls (which leave long random gaps — e.g. ~100 steps between samples at
        1%), this guarantees data appears on the first probed step and at a
        steady cadence. The schedule is derived purely from ``_step_cycle``
        (which advances one-for-one on every rank), so all ranks pick the same
        step and the host RNG is never touched. Cached per cycle so it stays
        stable across gradient-accumulation micro-steps.
        """
        if not self.finalized or not self._auto_plan:
            return
        cycle = getattr(self, "_step_cycle", 0)
        if self._planned_cycle == cycle:
            return
        self._planned_cycle = cycle
        self._sampled_mods_this_step = set()

        period = self._sample_period()
        self.sampled_step = (cycle % period) == 0

    def should_sample(self, mod, stage: Optional[str] = None) -> bool:
        if not self.finalized:
            self.register_mod(mod)
            return False

        if not self.sampled_step:
            return False

        # Time anchor: first pre hook in the step (pairs with its post hook).
        if stage and stage.startswith("pre") and self.offset() == 0:
            return True

        # On a sampled step, each layer is hit independently with probability
        # ``layer_rate`` (1.0 = full snapshot). The decision is a deterministic
        # hash of (step, layer) — it *looks* random and varies per layer/step,
        # yet is reproducible and identical across ranks, and never perturbs the
        # host RNG.
        if self.layer_rate >= 1.0:
            return True
        cycle = getattr(self, "_step_cycle", 0)
        return (
            _stable_unit_float("hook", cycle, self._module_display_name(mod))
            < self.layer_rate
        )

    def set_sampling_mode(self, expr):
        """Set sampling rate (``rate`` or ``rate:layer_rate``; ``mode`` is accepted
        for back-compat but only ``random`` sampling exists).

        ``rate`` sets the *step* sampling density: one step in every
        ``round(1/rate)`` is sampled, evenly spaced (deterministic per step, so
        ranks stay aligned). Within a sampled step each layer is recorded with
        probability ``layer_rate`` (default 1.0 = full snapshot), decided by a
        deterministic (step, layer) hash. Open ``pre`` stages always receive a
        matching ``post`` hook.

        Examples
        --------

        >>> tracer = TorchProbe()
        >>> tracer.mode, tracer.rate
        ('random', 0.05)

        >>> tracer.set_sampling_mode("random:0.1")
        >>> tracer.mode, tracer.rate
        ('random', 0.1)

        >>> tracer.set_sampling_mode("random:0.1:0.3")
        >>> (tracer.mode, tracer.rate, tracer.layer_rate)
        ('random', 0.1, 0.3)

        >>> tracer.set_sampling_mode("0.5")  # bare rate also works
        >>> tracer.mode, tracer.rate
        ('random', 0.5)

        >>> tracer.set_sampling_mode("invalid:1.5")
        >>> tracer.mode, tracer.rate
        ('random', 0.05)
        """
        # Force a re-plan so a hot rate switch takes effect next step.
        self._planned_cycle = None
        self.mode = "random"
        try:
            parts = expr.split(":")
            # First field may be a legacy mode name (ignored) or a bare rate:
            # treat a non-numeric leading token as a mode alias and skip it.
            rate_idx = 0 if _is_float(parts[0]) else 1
            rate = parts[rate_idx]
            layer_idx = rate_idx + 1

            self.rate = float(rate) if 0 < float(rate) <= 1 else DEFAULT_SAMPLE_RATE
            if len(parts) > layer_idx and 0 < float(parts[layer_idx]) <= 1:
                self.layer_rate = float(parts[layer_idx])
            else:
                self.layer_rate = 1.0
        except (ValueError, IndexError):
            print(f"Invalid sampling expression: {expr}. Using default settings.")
            self.rate = DEFAULT_SAMPLE_RATE
            self.layer_rate = 1.0


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
        self.curr_step = step.micro_step
        self.pending = []
        self._open_spans = {}
        self._train_step_cm = None
        self._step_cycle = 0
        self.shadow_step = False
        self._step_wall_started_at: Optional[float] = None
        self._backward_wall_start: dict[int, float] = {}
        # Backward grad hooks run on autograd worker threads where the step
        # coordinate is thread-local and reads as 0. Capture the snapshot on the
        # main thread when hooks are armed (during forward) so ``post backward``
        # rows carry the correct local_step instead of 0.
        self._backward_step_snap: dict[int, Any] = {}
        # GPU-event timing reads deferred off the sampled step's critical path
        # (see ``_drain_deferred``); each item is a ``DelayedRecord`` tagged with
        # ``_defer_cycle``.
        self._deferred: list = []
        # ``current_role()`` rescans the whole environment on every call (~70us);
        # the role is constant within a step, so resolve it once per step (see
        # ``_mark_step_wall_start``) and reuse it for every stamped row.
        self._role_this_step: str = current_role()

        super().__init__(
            tracepy=config.tracepy,
            sync=config.sync,
            rate=config.rate,
            layer_rate=config.layer_rate,
            exprs=config.exprs,
        )

    def _stamp_step_role(self, record, snapshot=None) -> None:
        """Fill step coordinate and parallel role on a torch_trace record.

        ``snapshot`` overrides the live step coordinate — required for backward
        rows, whose grad hooks fire on autograd threads where the thread-local
        coordinate reads 0 (see ``_backward_step_snap``).
        """
        snap = snapshot if snapshot is not None else step.snapshot()
        for key, value in row_fields(snap).items():
            setattr(record, key, value)
        record.role = self._role_this_step

    def _refresh_shadow_flag(self) -> None:
        self.shadow_step = shadow_step_in_cycle(
            self._step_cycle,
            self.config.shadow_normal,
            self.config.shadow_baseline,
        )

    def _advance_step_cycle_for_next(self) -> None:
        self._step_cycle += 1
        self._refresh_shadow_flag()

    def _mark_step_wall_start(self) -> None:
        self._step_wall_started_at = time.perf_counter()
        # Refresh the per-step role cache (runs once per step, on every branch:
        # sampled, non-sampled, and shadow) so a runtime ``set_role`` / late env
        # is picked up next step without paying the env scan on every row.
        self._role_this_step = current_role()

    def _record_step_timing(self, *, is_shadow: bool) -> None:
        """Persist wall-clock step duration for probed vs shadow comparison."""
        if self._step_wall_started_at is None:
            return
        duration = time.perf_counter() - self._step_wall_started_at
        record = TorchStepTiming(
            step_duration_sec=duration,
            is_shadow=1 if is_shadow else 0,
            sampled=0 if is_shadow else (1 if self.sampled_step else 0),
            shadow_normal=self.config.shadow_normal,
            shadow_baseline=self.config.shadow_baseline,
            sample_rate=self.rate,
            sample_mode=self.mode,
        )
        self._stamp_step_role(record)
        try:
            record.save()
        except Exception as e:
            logger.debug("failed to save torch step timing: %s", e)

    def _hooks_dispatch_active(self) -> bool:
        """Whether module/optimizer trace hooks should run this step (not just wall timing)."""
        if not self.finalized:
            return True
        # Plan at the first hook of the step so non-sampled steps short-circuit
        # every mode (``ordered`` and ``random`` alike).
        self._ensure_step_plan()
        return self.sampled_step

    def pre_forward_hook(self, m, i):
        if self.shadow_step or not self._hooks_dispatch_active():
            return
        super().pre_forward_hook(m, i)

    def post_forward_hook(self, m, i, o):
        if self.shadow_step or not self._hooks_dispatch_active():
            return
        super().post_forward_hook(m, i, o)
        if self.config.backward:
            self._attach_backward_tensor_hooks(m, i, o)

    def pre_backward_hook(self, m, i):
        if self.shadow_step or not self._hooks_dispatch_active():
            return
        super().pre_backward_hook(m, i)

    def post_backward_hook(self, m, i, o):
        if self.shadow_step or not self._hooks_dispatch_active():
            return
        super().post_backward_hook(m, i, o)

    def pre_step_hook(self, optimizer, args, kwargs):
        if self.shadow_step:
            return
        if self.enabled and self.finalized:
            self._begin_train_step_span(optimizer=optimizer)
        if not self._hooks_dispatch_active():
            return
        super().pre_step_hook(optimizer, args, kwargs)

    def _should_emit_trace_span(self, mod, span_phase: str) -> bool:
        if not self.config.trace_spans:
            return False
        if is_training_phase(span_phase):
            from probing.tracing.hooks import owns_training_phases

            if owns_training_phases(module=mod):
                return False
        return True

    def _begin_train_step_span(self, optimizer=None) -> None:
        if not self.config.trace_spans:
            return
        if self._train_step_cm is not None:
            return
        from probing.tracing.hooks import owns_training_phases

        if optimizer is not None and owns_training_phases(optimizer=optimizer):
            return
        handle = span(phase=OPTIMIZER, source="torch_probe")
        handle.__enter__()
        self._train_step_cm = handle

    def _end_train_step_span(self) -> None:
        if self._train_step_cm is None:
            return
        inner = getattr(self._train_step_cm, "_inner", None)
        if inner is not None and getattr(inner, "_reentrant", False):
            self._train_step_cm = None
            return
        self._train_step_cm.__exit__(None, None, None)
        self._train_step_cm = None

    def _post_stage_for_pre(self, pre_stage: str) -> str:
        if pre_stage.startswith("pre "):
            return "post " + pre_stage[4:]
        return pre_stage

    def _open_backward_timer(self, mod) -> None:
        """Open a backward span + start the timer (no ``pre backward`` row).

        Backward is timed on the *same clock as forward*: on an async backend a
        GPU event is recorded here (and another when the module's backward
        finishes) so we capture real backward GPU time — not just the CPU
        dispatch interval, which is near-zero on CUDA/MPS and made backward
        vanish from the flamegraph. CPU backends fall back to a wall clock.
        """
        span_key = (id(mod), "backward")
        if span_key in self._open_spans:
            return
        module_name_str = self._module_display_name(mod)

        span_cm = None
        span_phase = infer_from_stage("pre backward")
        emit_trace_span = self._should_emit_trace_span(mod, span_phase)
        if emit_trace_span:
            span_kwargs = dict(
                module=module_name_str,
                stage="backward",
                seq=self.offset(),
                source="torch_probe",
            )
            if is_training_phase(span_phase):
                span_cm = span(module_name_str, phase=span_phase, **span_kwargs)
            else:
                span_cm = span(module_name_str, **span_kwargs)
            span_cm.__enter__()

        # Slots 3/4 (allocated, max_allocated) are unused for backward.
        self._open_spans[span_key] = (span_cm, mod, "pre backward", 0.0, 0.0)
        self._backward_wall_start[id(mod)] = self._backward_timer_start()

    def _backward_timer_start(self):
        """Return a start marker for a module's backward timing.

        On an async backend this is a *recorded* GPU event (so the duration is
        measured on the GPU clock, matching forward); otherwise a
        ``perf_counter`` float for CPU wall timing.
        """
        if self.use_gpu_events:
            try:
                event = _backend_event(enable_timing=True)
                if event is not None:
                    event.record()
                    return event
                self.use_gpu_events = False
            except (AttributeError, TypeError):
                self.use_gpu_events = False
        return time.perf_counter()

    def _save_post_backward(self, mod, start_marker) -> None:
        """Persist a single ``post backward`` row (flamegraph input).

        ``start_marker`` comes from :meth:`_backward_timer_start`: a GPU event
        (async backends → time on the GPU clock, deferred like forward so the
        sampled step never blocks) or a ``perf_counter`` float (CPU wall time).
        """
        module_name_str = self._module_display_name(mod)
        span_key = (id(mod), "backward")
        entry = self._open_spans.pop(span_key, None)
        if entry is not None:
            span_cm = entry[0]
            if span_cm is not None:
                try:
                    span_cm.__exit__(None, None, None)
                except Exception:
                    pass
        self._release_timers(span_key)

        record = mem_stats()
        # Use the forward-time snapshot: this may run on an autograd thread whose
        # thread-local step coordinate reads 0, which would filter the row out of
        # the recent-step flamegraph window.
        self._stamp_step_role(record, self._backward_step_snap.pop(id(mod), None))
        record.seq = self.offset()
        record.module = module_name_str
        record.stage = "post backward"
        record.time_offset = 0.0

        if start_marker is not None and not isinstance(start_marker, float):
            # GPU-event start → record the matching end event and time on the
            # GPU clock. Reading ``elapsed_time`` needs the events complete, so
            # defer it (same machinery as forward) to keep the step off-critical.
            end_event = None
            try:
                end_event = _backend_event(enable_timing=True)
                if end_event is not None:
                    end_event.record()
            except (AttributeError, TypeError):
                end_event = None
            if end_event is not None:
                deferred = DelayedRecord(record, (start_marker, end_event))
                deferred._defer_cycle = self._step_cycle
                self._deferred.append(deferred)
                return
            record.duration = 0.0
        elif isinstance(start_marker, float):
            record.duration = max(time.perf_counter() - start_marker, 0.0)
        else:
            record.duration = 0.0
        try:
            DelayedRecord(record, None).save()
        except Exception as e:
            logger.debug("failed to save post backward trace: %s", e)

    def _complete_post_stage(self, mod, post_stage: str) -> None:
        """Finish a pre/post pair (used for normal post hooks and step-end cleanup)."""
        mapped_stage = STAGEMAP[post_stage]
        span_key = (id(mod), mapped_stage)

        record = mem_stats()
        self._stamp_step_role(record)
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
            if span_cm is not None:
                span_cm.__exit__(None, None, None)
        self.pending.append(DelayedRecord(record, events))

    def _finish_open_stages(self) -> None:
        """Close any pre stages that never received a post hook this step."""
        while self._open_spans:
            span_key = next(iter(self._open_spans))
            entry = self._open_spans[span_key]
            mod, pre_stage = entry[1], entry[2]
            post_stage = self._post_stage_for_pre(pre_stage)
            if pre_stage == "pre backward":
                started = self._backward_wall_start.pop(id(mod), None)
                try:
                    # ``started`` may be a GPU event or wall-clock float; both are
                    # understood by ``_save_post_backward``. ``None`` still closes
                    # the span (guarantees this loop makes progress).
                    self._save_post_backward(mod, started)
                except Exception as e:
                    logger.debug(
                        "error completing orphan backward for %s: %s",
                        self._module_display_name(mod),
                        e,
                    )
                    self._open_spans.pop(span_key, None)
                continue
            try:
                self._complete_post_stage(mod, post_stage)
            except Exception as e:
                logger.debug("error completing open stage %s: %s", pre_stage, e)
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
        self._backward_wall_start.clear()
        self._backward_step_snap.clear()

    def _attach_backward_tensor_hooks(self, mod, inputs, output) -> None:
        """Time a module's *own* backward via output/input grad hooks.

        ``grad_output`` is ready just *before* the module's backward runs;
        ``grad_input`` just *after*. So the module's backward compute is the
        interval between the two, measured with plain tensor ``register_hook``
        callbacks (inplace-safe, unlike module backward hooks). Modules whose
        input needs no grad (e.g. the first layer) can't be measured → ~0.
        """
        if not self.enabled or not self.finalized or self.shadow_step:
            return
        # Backward follows forward: attach only for modules whose forward was
        # recorded this step, so forward/backward coverage stays in lockstep.
        if not self.sampled_step or id(mod) not in self._sampled_mods_this_step:
            return
        span_key = (id(mod), "backward")
        if span_key in self._open_spans:
            return
        out_tensors = [
            t for t in _iter_tensors(output) if getattr(t, "requires_grad", False)
        ]
        if not out_tensors:
            return
        in_tensors = [
            t for t in _iter_tensors(inputs) if getattr(t, "requires_grad", False)
        ]

        # ``_backward_wall_start[id(mod)]`` (set by ``_open_backward_timer``) is the
        # single start timestamp; the orphan path in ``_finish_open_stages`` reads
        # it too. ``state`` only tracks how many grad callbacks are still pending.
        state = {
            "started": False,
            "out_left": len(out_tensors),
            "in_left": len(in_tensors),
        }
        # Snapshot the step coordinate now (main/forward thread); grad callbacks
        # below run on autograd threads where it would read 0.
        self._backward_step_snap[id(mod)] = step.snapshot()

        def _finalize() -> None:
            if span_key not in self._open_spans:
                return
            start = self._backward_wall_start.pop(id(mod), None)
            self._save_post_backward(mod, start)

        def _on_out_grad(grad):
            # grad_output ready → this module's backward is about to start.
            if not state["started"]:
                state["started"] = True
                self._open_backward_timer(mod)
            state["out_left"] -= 1
            # No grad-requiring input → grad_input never fires → close at ~0.
            if state["out_left"] == 0 and state["in_left"] == 0:
                _finalize()
            return grad

        def _on_in_grad(grad):
            # grad_input ready → this module's backward just finished.
            state["in_left"] -= 1
            if state["in_left"] == 0 and id(mod) in self._backward_wall_start:
                _finalize()
            return grad

        for tensor in out_tensors:
            tensor.register_hook(_on_out_grad)
        for tensor in in_tensors:
            tensor.register_hook(_on_in_grad)

    def log_module_stage(self, stage, mod, force=False) -> None:
        if not self.enabled or self.shadow_step:
            return

        mapped_stage = STAGEMAP[stage]
        span_key = (id(mod), mapped_stage)
        pairing = stage.startswith("post") and span_key in self._open_spans

        if not force and not pairing and not self.should_sample(mod, stage):
            return

        # Remember modules whose forward is recorded so backward hooks can mirror
        # the exact same coverage this step.
        if stage == "pre forward":
            self._sampled_mods_this_step.add(id(mod))

        record = mem_stats()
        self._stamp_step_role(record)
        record.seq = self.offset()
        module_name_str = self._module_display_name(mod)
        record.module = module_name_str
        record.stage = stage
        span_phase = infer_from_stage(stage)

        emit_trace_span = self._should_emit_trace_span(mod, span_phase)

        if stage.startswith("pre"):
            record.time_offset = self.begin_timing(mod, stage)
            span_cm = None
            if emit_trace_span:
                span_kwargs = dict(
                    module=module_name_str,
                    stage=mapped_stage,
                    seq=record.seq,
                    source="torch_probe",
                )
                if is_training_phase(span_phase):
                    span_cm = span(module_name_str, phase=span_phase, **span_kwargs)
                else:
                    span_cm = span(module_name_str, **span_kwargs)
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

    @staticmethod
    def _deferred_events_ready(dr) -> bool:
        """True when a deferred record's GPU end-event has completed (or it has none).

        ``event.query()`` is a non-blocking readiness check, so this never stalls.
        """
        events = dr.events
        if not events:
            return True
        end = events[1]
        if end is None:
            return True
        try:
            return bool(end.query())
        except Exception:
            # query() unsupported/failed → let save() attempt the read.
            return True

    @staticmethod
    def _force_event_complete(dr) -> None:
        events = dr.events
        if not events:
            return
        end = events[1]
        if end is None:
            return
        try:
            end.synchronize()
        except Exception:
            pass

    def _drain_deferred(self) -> None:
        """Persist GPU-event timings stashed by earlier sampled steps.

        Reading ``elapsed_time`` needs the events to have completed; doing that on
        the sampled step forces a blocking ``synchronize`` on an async backend
        (CUDA/MPS) — that stall is exactly what inflates sampled-step wall time.
        Instead we stash the (record, events) and drain them here on later
        optimizer steps once the GPU has naturally caught up.

        A record is only read after it has aged ``_DEFER_MIN_SETTLE`` optimizer
        steps *and* its end event reports ready (``event.query()``). The settle
        window matters most for backward events, which are recorded near the end
        of a step: draining them the very next step (while the kernels they
        bracket are still in flight) makes each ``elapsed_time`` force its own
        blocking commit — dozens of ms per sampled step. Letting them age a couple
        of steps turns every read into a free cached lookup, exactly like forward
        events already behave. Entries older than ``_DEFER_MAX_LAG`` cycles are
        force-completed (rare).
        """
        if not self._deferred:
            return
        cutoff = self._step_cycle - _DEFER_MAX_LAG
        settle = self._step_cycle - _DEFER_MIN_SETTLE
        remaining = []
        for dr in self._deferred:
            defer_cycle = getattr(dr, "_defer_cycle", self._step_cycle)
            force = defer_cycle <= cutoff
            # Only read events that have had a few optimizer steps to settle on
            # the GPU. Reading ``elapsed_time`` too soon (e.g. the very next step,
            # while the backward kernels those events bracket are still in flight)
            # forces a blocking commit — that stall is the sampled-step overhead.
            # Letting them age a couple of steps means the reads hit already
            # completed work for free, exactly like forward events naturally do.
            settled = defer_cycle <= settle
            if force or (settled and self._deferred_events_ready(dr)):
                if force:
                    self._force_event_complete(dr)
                dr.save()
            else:
                remaining.append(dr)
        self._deferred = remaining

    def post_step_hook(self, opt, args, kwargs):
        if not self.enabled:
            return
        if not self.finalized:
            self.finalize_discovery()
            self._step_cycle = 0
            self._refresh_shadow_flag()
            self.curr_step = step.micro_step
            self._mark_step_wall_start()
            self._begin_train_step_span(optimizer=opt)
            return

        # Reclaim earlier sampled steps' GPU timings every optimizer step, off
        # their critical path (runs for shadow / non-sampled / sampled alike).
        self._drain_deferred()

        if self.shadow_step:
            self._end_train_step_span()
            self.curr_step = step.micro_step
            self._cleanup_step_resources()
            self.step_start = None
            self._record_step_timing(is_shadow=True)
            self._advance_step_cycle_for_next()
            self._mark_step_wall_start()
            return

        self._end_train_step_span()
        self.curr_step = step.micro_step

        if not self.sampled_step:
            self._module_call_offset = 0
            self._current_module = None
            self._current_stage = None
            self._cleanup_step_resources()
            self.step_start = None
            self.pending.clear()
            self._record_step_timing(is_shadow=False)
            self._advance_step_cycle_for_next()
            self._mark_step_wall_start()
            return

        super().post_step_hook(opt, args, kwargs)

        self._finish_open_stages()

        # ``pre backward`` rows carry no duration (backward is wall-clock timed) —
        # drop them so the table is not flooded with 0s.
        records = [p for p in self.pending if p.record.stage != "pre backward"]
        self.pending.clear()

        if self.use_gpu_events:
            # Async backend (CUDA/MPS): reading ``elapsed_time`` now would force a
            # blocking ``synchronize`` on the sampled step — the stall that
            # inflates its wall time. Defer instead and drain on later optimizer
            # steps once the GPU has caught up (see ``_drain_deferred``).
            cycle = self._step_cycle
            for pending in records:
                pending._defer_cycle = cycle
                self._deferred.append(pending)
        else:
            # CPU timing: no async work to wait on, so persist immediately.
            for pending in records:
                pending.save()

        # trace Python variables
        self.trace_variables()

        self._cleanup_step_resources()
        self.step_start = None
        self._record_step_timing(is_shadow=False)
        self._advance_step_cycle_for_next()
        self._mark_step_wall_start()


def set_sampling_mode(mode):
    try:
        for obj in _live_torch_probes():
            obj.set_sampling_mode(mode)
    except Exception as e:
        logger.debug("error setting sampling mode: %s", e)

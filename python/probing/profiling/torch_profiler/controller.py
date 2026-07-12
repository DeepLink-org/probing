"""ProfilerController: short-window torch.profiler with SQL row materialization."""

from __future__ import annotations

import json
import logging
import os
import tempfile
import threading
import time
from typing import Any, Optional

from .adaptor import compile_from_profiler
from .session_store import CaptureRecord, get_session_store

logger = logging.getLogger(__name__)

try:
    import torch

    HAS_TORCH = True
except ImportError:
    HAS_TORCH = False
    torch = None  # type: ignore[assignment,misc]


def _now_us() -> int:
    return int(time.time() * 1_000_000)


class ProfilerController:
    """Drive torch.profiler for N optimizer steps and publish hotspot rows."""

    def __init__(self) -> None:
        self._lock = threading.RLock()
        self._profiler: Any = None
        self._hook_handle: Any = None
        self._steps_target = 0
        self._step_count = 0
        self._trigger = ""
        self._started_at_us = 0
        self._cached_timeline: Optional[str] = None
        self._timeline_exported = False
        self._running = False

    @property
    def is_running(self) -> bool:
        with self._lock:
            return self._running

    def status(self) -> dict[str, Any]:
        with self._lock:
            latest = get_session_store().latest_capture_id()
            return {
                "running": self._running,
                "steps_target": self._steps_target,
                "steps_completed": self._step_count,
                "trigger": self._trigger,
                "latest_capture_id": latest,
            }

    def start(self, *, steps: int = 1, trigger: str = "manual") -> None:
        if not HAS_TORCH:
            raise ImportError("PyTorch is not installed")

        steps = max(int(steps), 1)
        with self._lock:
            if self._running:
                raise RuntimeError("profiler already running")
            self._steps_target = steps
            self._step_count = 0
            self._trigger = trigger
            self._started_at_us = _now_us()
            self._cached_timeline = None
            self._timeline_exported = False
            self._running = True

            activities = [torch.profiler.ProfilerActivity.CPU]
            if torch.cuda.is_available():
                activities.append(torch.profiler.ProfilerActivity.CUDA)

            self._profiler = torch.profiler.profile(
                record_shapes=True,
                with_stack=True,
                with_flops=True,
                activities=activities,
                on_trace_ready=None,
            )

            from torch.optim.optimizer import register_optimizer_step_post_hook

            controller = self

            def profiler_step_hook(optimizer, *args, **kwargs):
                del optimizer, args, kwargs
                with controller._lock:
                    if controller._profiler is None or not controller._running:
                        return
                    if controller._step_count == 0:
                        controller._profiler.__enter__()
                        logger.info(
                            "torch profiler started (trigger=%s, steps=%d)",
                            controller._trigger,
                            controller._steps_target,
                        )
                    if controller._step_count >= controller._steps_target:
                        return
                    try:
                        controller._profiler.step()
                        controller._step_count += 1
                        if controller._step_count >= controller._steps_target:
                            controller._finalize_capture(status="completed")
                    except RuntimeError as exc:
                        controller._finalize_capture(status="failed", error=str(exc))

            self._hook_handle = register_optimizer_step_post_hook(profiler_step_hook)

    def stop(self) -> Optional[str]:
        """Stop early; returns capture_id when a capture was materialized."""
        with self._lock:
            if not self._running:
                return get_session_store().latest_capture_id()
            return self._finalize_capture(status="completed")

    def summary(self) -> None:
        with self._lock:
            profiler = self._profiler
        if profiler is None:
            logger.info("profiler not initialized")
            return
        try:
            events = profiler.events()
            event_list = list(events) if events else []
            if event_list:
                table = profiler.key_averages().table(
                    sort_by="self_cuda_time_total", row_limit=10
                )
                logger.info("profiler collected %d events\n%s", len(event_list), table)
            else:
                logger.info(
                    "profiler has no events (steps %d/%d)",
                    self._step_count,
                    self._steps_target,
                )
        except Exception as exc:
            logger.debug("profiler summary failed: %s", exc)

    def export_timeline(self) -> Optional[str]:
        with self._lock:
            if self._timeline_exported and self._cached_timeline is not None:
                return self._cached_timeline
            profiler = self._profiler
        if profiler is None:
            return None
        try:
            tmp_fd, tmp_path = tempfile.mkstemp(suffix=".json", text=True)
            os.close(tmp_fd)
            try:
                profiler.export_chrome_trace(tmp_path)
                with open(tmp_path, encoding="utf-8") as handle:
                    trace_json = handle.read()
                parsed = json.loads(trace_json)
                if not parsed.get("traceEvents"):
                    return None
                with self._lock:
                    self._cached_timeline = trace_json
                    self._timeline_exported = True
                return trace_json
            finally:
                try:
                    os.unlink(tmp_path)
                except OSError:
                    pass
        except Exception as exc:
            logger.debug("timeline export failed: %s", exc)
            return None

    def _remove_hook(self) -> None:
        if self._hook_handle is None:
            return
        try:
            from torch.optim.optimizer import remove_optimizer_step_post_hook

            remove_optimizer_step_post_hook(self._hook_handle)
        except Exception as exc:
            logger.debug("failed to remove optimizer hook: %s", exc)
        self._hook_handle = None

    def _finalize_capture(
        self, *, status: str = "completed", error: str = ""
    ) -> Optional[str]:
        if not self._running and self._profiler is None:
            return get_session_store().latest_capture_id()

        self._remove_hook()
        profiler = self._profiler
        started = self._started_at_us
        trigger = self._trigger
        steps_done = self._step_count
        self._running = False
        self._profiler = None

        capture: Optional[CaptureRecord] = None
        if profiler is not None:
            try:
                profiler.__exit__(None, None, None)
            except Exception as exc:
                logger.debug("profiler exit: %s", exc)
                if status == "completed":
                    status = "failed"
                    error = error or str(exc)
            try:
                capture, hotspots = compile_from_profiler(
                    profiler,
                    trigger=trigger,
                    steps_profiled=steps_done,
                    started_at_us=started,
                    status=status,
                    error=error,
                )
                get_session_store().add_capture(capture, hotspots)
                logger.info(
                    "profile capture %s: %d hotspots, step=%d status=%s",
                    capture.capture_id,
                    len(hotspots),
                    capture.local_step,
                    capture.status,
                )
            except Exception as exc:
                logger.warning("failed to compile profile capture: %s", exc)
        return capture.capture_id if capture is not None else None


_CONTROLLER: Optional[ProfilerController] = None
_CONTROLLER_LOCK = threading.Lock()


def get_controller() -> ProfilerController:
    global _CONTROLLER
    with _CONTROLLER_LOCK:
        if _CONTROLLER is None:
            _CONTROLLER = ProfilerController()
        return _CONTROLLER


def reset_controller_for_tests() -> None:
    """Drop singleton controller between tests (not for production)."""
    global _CONTROLLER
    with _CONTROLLER_LOCK:
        if _CONTROLLER is not None:
            with _CONTROLLER._lock:
                if _CONTROLLER._running:
                    _CONTROLLER._finalize_capture(status="failed", error="test reset")
                else:
                    _CONTROLLER._remove_hook()
                    _CONTROLLER._profiler = None
        _CONTROLLER = None


def reset_torch_profiler_for_tests() -> None:
    """Reset session store + controller (test helper)."""
    from .session_store import reset_session_store_for_tests

    reset_controller_for_tests()
    reset_session_store_for_tests()


def profiler_status() -> dict[str, Any]:
    return get_controller().status()

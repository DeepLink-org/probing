"""In-process store for profile captures and hotspot rows (no memtable)."""

from __future__ import annotations

import os
import threading
from dataclasses import dataclass, field
from typing import Optional


def _max_sessions() -> int:
    raw = os.environ.get("PROBING_TORCH_PROFILER_MAX_SESSIONS", "8").strip()
    try:
        value = int(raw)
    except ValueError:
        return 8
    return max(value, 1)


@dataclass
class CaptureRecord:
    capture_id: str
    local_step: int = -1
    global_step: int = -1
    rank: int = -1
    world_size: int = -1
    role: str = ""
    trigger: str = ""
    steps_profiled: int = 0
    wall_us: int = 0
    started_at_us: int = 0
    ended_at_us: int = 0
    status: str = "running"
    truncated: bool = False
    event_count: int = 0
    error: str = ""


@dataclass
class HotspotRecord:
    capture_id: str
    local_step: int = -1
    global_step: int = -1
    rank: int = -1
    bucket_kind: str = "other"
    bucket_name: str = ""
    self_us: int = 0
    wall_us: int = 0
    calls: int = 0
    pct_of_capture: float = 0.0
    module_hint: str = ""


@dataclass
class SessionStore:
    """Bounded in-memory captures + hotspot fact rows."""

    max_sessions: int = field(default_factory=_max_sessions)
    _captures: list[CaptureRecord] = field(default_factory=list)
    _hotspots: list[HotspotRecord] = field(default_factory=list)
    _lock: threading.RLock = field(default_factory=threading.RLock)

    def add_capture(
        self,
        capture: CaptureRecord,
        hotspots: list[HotspotRecord],
    ) -> None:
        with self._lock:
            self._captures.append(capture)
            self._hotspots.extend(hotspots)
            overflow = len(self._captures) - self.max_sessions
            if overflow > 0:
                drop_ids = {c.capture_id for c in self._captures[:overflow]}
                self._captures = self._captures[overflow:]
                self._hotspots = [
                    h for h in self._hotspots if h.capture_id not in drop_ids
                ]

    def captures(self) -> list[CaptureRecord]:
        with self._lock:
            return list(self._captures)

    def hotspots(self) -> list[HotspotRecord]:
        with self._lock:
            return list(self._hotspots)

    def latest_capture_id(self) -> Optional[str]:
        with self._lock:
            if not self._captures:
                return None
            return self._captures[-1].capture_id

    def clear(self) -> None:
        with self._lock:
            self._captures.clear()
            self._hotspots.clear()


_STORE: Optional[SessionStore] = None
_STORE_LOCK = threading.Lock()


def get_session_store() -> SessionStore:
    global _STORE
    with _STORE_LOCK:
        if _STORE is None:
            _STORE = SessionStore()
        return _STORE


def reset_session_store_for_tests() -> None:
    """Clear captures/hotspots between tests (not for production)."""
    global _STORE
    with _STORE_LOCK:
        if _STORE is not None:
            _STORE.clear()

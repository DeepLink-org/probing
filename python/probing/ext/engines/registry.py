"""In-process registry for inference engine scrape targets."""

from __future__ import annotations

import threading
import time
from dataclasses import asdict, dataclass, field
from typing import Any

from probing.ext.engines.sglang import DEFAULT_METRICS_PATH, resolve_metrics_path


@dataclass
class EngineRegistration:
    engine_id: str
    engine_type: str
    router_addr: str
    metrics_path: str = DEFAULT_METRICS_PATH
    framework: str = "agentic-rl"
    labels: dict[str, str] = field(default_factory=dict)
    registered_at_ns: int = field(default_factory=lambda: time.time_ns())
    last_scrape_at_ns: int | None = None
    last_scrape_error: str | None = None
    last_normalized: dict[str, float] = field(default_factory=dict)

    @property
    def metrics_url(self) -> str:
        from probing.ext.engines.sglang import build_metrics_url

        return build_metrics_url(self.router_addr, self.metrics_path)

    def to_dict(self) -> dict[str, Any]:
        payload = asdict(self)
        payload["metrics_url"] = self.metrics_url
        payload["status"] = "healthy" if self.last_scrape_error is None else "degraded"
        return payload


_lock = threading.Lock()
_engines: dict[str, EngineRegistration] = {}


def register_engine(
    *,
    router_addr: str,
    engine_id: str = "inference-engine",
    engine_type: str = "sglang",
    metrics_path: str | None = None,
    framework: str = "agentic-rl",
    labels: dict[str, str] | None = None,
    auto_scrape: bool = True,
) -> EngineRegistration:
    """Register an inference engine metrics endpoint."""

    registration = EngineRegistration(
        engine_id=engine_id,
        engine_type=engine_type,
        router_addr=router_addr,
        metrics_path=resolve_metrics_path(metrics_path),
        framework=framework,
        labels=dict(labels or {}),
    )
    with _lock:
        _engines[engine_id] = registration
    if auto_scrape:
        from probing.ext.engines.scraper import ensure_scraper_running

        ensure_scraper_running()
    return registration


def register_slime_sglang_router(
    router_addr: str | None,
    *,
    engine_id: str = "sglang-router",
    framework: str = "slime",
    labels: dict[str, str] | None = None,
    auto_scrape: bool = True,
) -> EngineRegistration | None:
    """Register Slime's ``sglang_router`` metrics endpoint.

    Slime returns ``http://{router_ip}:{router_port}`` from
    ``RolloutManager.get_metrics_router_addr()``. Metrics are scraped from
    ``{router_addr}/engine_metrics``.
    """

    if not router_addr:
        return None
    merged_labels = {"integration": "slime", "source": "sglang_router"}
    if labels:
        merged_labels.update(labels)
    return register_engine(
        router_addr=router_addr,
        engine_id=engine_id,
        engine_type="sglang",
        metrics_path=DEFAULT_METRICS_PATH,
        framework=framework,
        labels=merged_labels,
        auto_scrape=auto_scrape,
    )


def unregister_engine(engine_id: str) -> bool:
    with _lock:
        return _engines.pop(engine_id, None) is not None


def get_engine(engine_id: str) -> EngineRegistration | None:
    with _lock:
        registration = _engines.get(engine_id)
        return registration


def list_engines() -> list[EngineRegistration]:
    with _lock:
        return list(_engines.values())


def update_scrape_result(
    engine_id: str,
    *,
    normalized: dict[str, float],
    scrape_error: str | None,
) -> None:
    with _lock:
        registration = _engines.get(engine_id)
        if registration is None:
            return
        registration.last_scrape_at_ns = time.time_ns()
        registration.last_scrape_error = scrape_error
        registration.last_normalized = dict(normalized)

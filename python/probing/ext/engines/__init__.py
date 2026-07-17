"""Inference engine metrics for Probing-RL.

Generic registration is framework-neutral. Concrete adapters (e.g. SGLang,
Slime ``sglang_router``) live under ``probing.ext.engines.sglang`` and
``register_slime_sglang_router``. Metrics paths differ by adapter::

    /engine_metrics  — Slime sglang_router / model gateway
    /metrics         — sglang.launch_server --enable-metrics
"""

from probing.ext.engines.registry import (
    get_engine,
    list_engines,
    register_engine,
    register_slime_sglang_router,
    unregister_engine,
)
from probing.ext.engines.scraper import ensure_scraper_running, scrape_all, scrape_engine

__all__ = [
    "ensure_scraper_running",
    "get_engine",
    "list_engines",
    "register_engine",
    "register_slime_sglang_router",
    "scrape_all",
    "scrape_engine",
    "unregister_engine",
]

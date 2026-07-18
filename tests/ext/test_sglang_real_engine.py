"""Integration test against a real ``sglang.launch_server`` with ``--enable-metrics``.

Skipped automatically when no server is reachable at ``SGLANG_ENGINE_URL``
(default ``http://127.0.0.1:30000``).
"""

from __future__ import annotations

import os
import urllib.error
import urllib.request

import pytest

from probing.ext.engines.mock_sglang_server import send_dummy_generate_requests
from probing.ext.engines.registry import get_engine, register_engine, unregister_engine
from probing.ext.engines.sglang import LAUNCH_SERVER_METRICS_PATH
from probing.ext.engines.scraper import scrape_engine

ENGINE_URL = os.environ.get("SGLANG_ENGINE_URL", "http://127.0.0.1:30000").rstrip("/")
ENGINE_ID = "sglang-real-test"


def _engine_reachable() -> bool:
    try:
        request = urllib.request.Request(
            f"{ENGINE_URL}/metrics",
            headers={"Accept": "text/plain"},
        )
        with urllib.request.urlopen(request, timeout=5) as response:
            return 200 <= response.status < 500
    except (urllib.error.URLError, TimeoutError, OSError):
        return False


@pytest.fixture(autouse=True)
def _cleanup_registry():
    unregister_engine(ENGINE_ID)
    yield
    unregister_engine(ENGINE_ID)


@pytest.mark.integration
@pytest.mark.skipif(not _engine_reachable(), reason=f"SGLang not reachable at {ENGINE_URL}")
def test_real_sglang_engine_metrics_scrape():
    registration = register_engine(
        router_addr=ENGINE_URL,
        engine_id=ENGINE_ID,
        engine_type="sglang",
        framework="standalone",
        metrics_path=LAUNCH_SERVER_METRICS_PATH,
        labels={"source": "real_sglang_test"},
        auto_scrape=False,
    )

    before = scrape_engine(registration)
    assert before["scrape_error"] is None, before.get("scrape_error")
    assert before["metrics_url"] == f"{ENGINE_URL}/metrics"

    responses = send_dummy_generate_requests(
        ENGINE_URL,
        count=2,
        max_new_tokens=16,
    )
    assert len(responses) == 2
    assert all("error" not in item for item in responses)

    after = scrape_engine(registration)
    assert after["scrape_error"] is None, after.get("scrape_error")
    normalized = after["normalized"]
    assert normalized, "expected at least one normalized metric"

    stored = get_engine(ENGINE_ID)
    assert stored is not None
    assert stored.last_scrape_error is None
    assert stored.last_normalized == normalized

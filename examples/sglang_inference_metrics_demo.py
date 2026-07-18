#!/usr/bin/env python3
"""Standalone SGLang inference metrics demo (no Slime).

Starts a lightweight mock SGLang HTTP server (or connects to an existing one),
registers it with Probing, sends dummy ``/generate`` requests, and prints the
Web UI URL for manual inspection.

Usage::

    # PROBING_PORT is required for browser access (otherwise only a Unix socket is started).
    PROBING=1 PROBING_PORT=8080 python examples/sglang_inference_metrics_demo.py

    # Use a real ``sglang.launch_server`` you already launched (metrics at /metrics):
    PROBING=1 PROBING_PORT=8080 python examples/sglang_inference_metrics_demo.py \\
        --engine-url http://127.0.0.1:30000

    # Slime router / model gateway (metrics at /engine_metrics):
    PROBING=1 PROBING_PORT=8080 python examples/sglang_inference_metrics_demo.py \\
        --engine-url http://127.0.0.1:30000 \\
        --metrics-path /engine_metrics

Then open the printed ``/inference`` page in your browser.
"""

from __future__ import annotations

import argparse
import os
import sys
import time
import urllib.error
import urllib.request

import probing.ext.engines as engines
from probing.ext.engines.mock_sglang_server import (
    send_dummy_generate_requests,
    start_mock_sglang_server,
)
from probing.ext.engines.sglang import (
    DEFAULT_METRICS_PATH,
    LAUNCH_SERVER_METRICS_PATH,
    resolve_metrics_path,
)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--engine-url",
        default="",
        help="Existing SGLang HTTP base URL. If omitted, a local mock server is started.",
    )
    parser.add_argument("--engine-id", default="sglang-standalone")
    parser.add_argument(
        "--metrics-path",
        default="",
        help=(
            "Prometheus scrape path. Defaults to /engine_metrics for mock servers and "
            "/metrics for external launch_server. Override with PROBING_ENGINE_METRICS_PATH."
        ),
    )
    parser.add_argument("--num-requests", type=int, default=8)
    parser.add_argument("--max-new-tokens", type=int, default=32)
    parser.add_argument("--request-latency-seconds", type=float, default=0.05)
    parser.add_argument(
        "--keep-alive-seconds",
        type=int,
        default=600,
        help="Keep process alive so the Probing Web UI can poll metrics.",
    )
    parser.add_argument(
        "--probing-port",
        type=int,
        default=int(os.environ.get("PROBING_PORT", "8080")),
    )
    return parser.parse_args()


def _resolve_demo_metrics_path(args: argparse.Namespace, *, using_mock: bool) -> str:
    if args.metrics_path.strip():
        return resolve_metrics_path(args.metrics_path.strip())
    if using_mock:
        return DEFAULT_METRICS_PATH
    return LAUNCH_SERVER_METRICS_PATH


def _probe_http_ui(port: int) -> bool:
    try:
        with urllib.request.urlopen(f"http://127.0.0.1:{port}/inference", timeout=2) as response:
            return 200 <= response.status < 500
    except (urllib.error.URLError, TimeoutError, OSError):
        return False


def _print_web_ui_unavailable_help(port: int) -> None:
    script = "examples/sglang_inference_metrics_demo.py"
    if __file__.endswith("sglang_inference_metrics_demo.py"):
        script = __file__
    print()
    print("*** Probing Web UI is not reachable on HTTP ***")
    print(
        "By default, PROBING=1 starts an in-process Unix socket server only. "
        "Set PROBING_PORT to also expose the HTTP dashboard."
    )
    print()
    print("Stop this process and restart with:")
    print(f"  PROBING=1 PROBING_PORT={port} python {script} \\")
    print("      [your other args...]")
    print()
    print("If you are on a remote worker/pod, also port-forward 8080 to your laptop.")


def main() -> None:
    args = parse_args()
    mock_server = None
    if args.engine_url:
        engine_url = args.engine_url.rstrip("/")
        print(f"Using existing SGLang server at {engine_url}")
    else:
        mock_server = start_mock_sglang_server()
        engine_url = mock_server.base_url
        print(f"Started mock SGLang server at {engine_url}")

    metrics_path = _resolve_demo_metrics_path(args, using_mock=mock_server is not None)

    registration = engines.register_engine(
        router_addr=engine_url,
        engine_id=args.engine_id,
        engine_type="sglang",
        framework="standalone",
        metrics_path=metrics_path,
        labels={"source": "mock_sglang" if mock_server else "external_sglang"},
        auto_scrape=True,
    )
    print(f"Registered engine {registration.engine_id} -> {registration.metrics_url}")

    print(f"Sending {args.num_requests} dummy /generate requests...")
    responses = send_dummy_generate_requests(
        engine_url,
        count=args.num_requests,
        max_new_tokens=args.max_new_tokens,
        latency_seconds=args.request_latency_seconds,
    )
    print(f"Received {len(responses)} responses")

    snapshot = engines.scrape_engine(registration)
    print("Latest normalized metrics:")
    for key, value in sorted(snapshot.get("normalized", {}).items()):
        print(f"  {key}: {value}")
    if snapshot.get("scrape_error"):
        print(f"Scrape error: {snapshot['scrape_error']}")

    inference_url = f"http://127.0.0.1:{args.probing_port}/inference"
    print()
    if _probe_http_ui(args.probing_port):
        print("Open the Probing Web UI to inspect live metrics:")
        print(f"  {inference_url}")
        print()
        print("Useful API checks:")
        print(
            f"  curl 'http://127.0.0.1:{args.probing_port}/apis/pythonext/engines/snapshot'"
        )
        print(f"  curl 'http://127.0.0.1:{args.probing_port}/apis/pythonext/engines/scrape'")
    else:
        _print_web_ui_unavailable_help(args.probing_port)
        if "PROBING_PORT" not in os.environ:
            print("(PROBING_PORT is not set in the environment.)")
        return 1

    if args.keep_alive_seconds > 0:
        print(f"Keeping alive for {args.keep_alive_seconds}s...")
        time.sleep(args.keep_alive_seconds)

    if mock_server is not None:
        mock_server.stop()


if __name__ == "__main__":
    sys.exit(main() or 0)

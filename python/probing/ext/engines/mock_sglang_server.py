"""Minimal standalone SGLang-like HTTP server for metrics demos and tests.

Implements the endpoints Probing scrapes without Slime or a GPU:

- ``GET /engine_metrics`` — Prometheus text (same shape as SGLang)
- ``POST /generate`` — accepts dummy generation requests and updates metrics
- ``GET /health_generate`` — startup health check used by SGLang clients
"""

from __future__ import annotations

import json
import threading
import time
from dataclasses import dataclass, field
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
@dataclass
class MockSGLangState:
    num_requests_running: int = 0
    num_queue_reqs: int = 0
    total_requests: int = 0
    total_output_tokens: int = 0
    gen_throughput: float = 0.0
    time_per_output_token: float = 0.015
    time_to_first_token: float = 0.08
    cache_hit_rate: float = 0.0
    token_usage: int = 0
    max_total_num_tokens: int = 16384
    lock: threading.Lock = field(default_factory=threading.Lock)

    def render_prometheus(self) -> str:
        with self.lock:
            return (
                "# TYPE sglang:num_requests_running gauge\n"
                f"sglang:num_requests_running {self.num_requests_running}\n"
                "# TYPE sglang:num_queue_reqs gauge\n"
                f"sglang:num_queue_reqs {self.num_queue_reqs}\n"
                "# TYPE sglang:gen_throughput gauge\n"
                f"sglang:gen_throughput {self.gen_throughput:.3f}\n"
                "# TYPE sglang:time_per_output_token gauge\n"
                f"sglang:time_per_output_token {self.time_per_output_token:.6f}\n"
                "# TYPE sglang:time_to_first_token gauge\n"
                f"sglang:time_to_first_token {self.time_to_first_token:.6f}\n"
                "# TYPE sglang:cache_hit_rate gauge\n"
                f"sglang:cache_hit_rate {self.cache_hit_rate:.4f}\n"
                "# TYPE sglang:token_usage gauge\n"
                f"sglang:token_usage {self.token_usage}\n"
                "# TYPE sglang:max_total_num_tokens gauge\n"
                f"sglang:max_total_num_tokens {self.max_total_num_tokens}\n"
                "# TYPE sglang:total_requests counter\n"
                f"sglang:total_requests {self.total_requests}\n"
            )

    def begin_request(self, output_tokens: int, latency_seconds: float) -> None:
        with self.lock:
            self.num_requests_running += 1
            self.total_requests += 1

        time.sleep(latency_seconds)

        with self.lock:
            self.num_requests_running = max(self.num_requests_running - 1, 0)
            self.total_output_tokens += output_tokens
            self.token_usage = min(
                self.token_usage + output_tokens,
                self.max_total_num_tokens,
            )
            elapsed = max(latency_seconds, 1e-6)
            self.gen_throughput = output_tokens / elapsed
            self.time_per_output_token = elapsed / max(output_tokens, 1)
            self.time_to_first_token = min(self.time_to_first_token, elapsed * 0.2)
            if self.total_requests > 1:
                self.cache_hit_rate = min(0.95, self.cache_hit_rate + 0.05)


def _make_handler(state: MockSGLangState) -> type[BaseHTTPRequestHandler]:
    class Handler(BaseHTTPRequestHandler):
        def log_message(self, format: str, *args) -> None:  # noqa: A003
            return

        def _send(self, code: int, body: str, content_type: str = "text/plain") -> None:
            encoded = body.encode("utf-8")
            self.send_response(code)
            self.send_header("Content-Type", content_type)
            self.send_header("Content-Length", str(len(encoded)))
            self.end_headers()
            self.wfile.write(encoded)

        def do_GET(self) -> None:  # noqa: N802
            if self.path.rstrip("/") == "/engine_metrics":
                self._send(200, state.render_prometheus())
                return
            if self.path.rstrip("/") in {"/health_generate", "/health"}:
                self._send(200, "ok\n")
                return
            self._send(404, "not found\n")

        def do_POST(self) -> None:  # noqa: N802
            if self.path.rstrip("/") != "/generate":
                self._send(404, "not found\n")
                return

            length = int(self.headers.get("Content-Length", "0") or 0)
            raw = self.rfile.read(length) if length else b"{}"
            try:
                payload = json.loads(raw.decode("utf-8") or "{}")
            except json.JSONDecodeError:
                payload = {}

            max_new_tokens = int(payload.get("max_new_tokens") or payload.get("max_tokens") or 16)
            sleep_seconds = float(payload.get("mock_latency_seconds") or 0.05)
            state.begin_request(max_new_tokens, sleep_seconds)
            response = {
                "text": "mock completion",
                "meta_info": {
                    "completion_tokens": max_new_tokens,
                    "prompt_tokens": 8,
                },
            }
            self._send(200, json.dumps(response), content_type="application/json")

    return Handler


@dataclass
class MockSGLangServer:
    host: str
    port: int
    state: MockSGLangState
    _httpd: ThreadingHTTPServer | None = None
    _thread: threading.Thread | None = None

    @property
    def base_url(self) -> str:
        return f"http://{self.host}:{self.port}"

    def start(self) -> None:
        handler = _make_handler(self.state)
        self._httpd = ThreadingHTTPServer((self.host, self.port), handler)
        self._thread = threading.Thread(
            target=self._httpd.serve_forever,
            name="mock-sglang-http",
            daemon=True,
        )
        self._thread.start()

    def stop(self) -> None:
        if self._httpd is not None:
            self._httpd.shutdown()
            self._httpd.server_close()
            self._httpd = None


def start_mock_sglang_server(host: str = "127.0.0.1", port: int = 0) -> MockSGLangServer:
    state = MockSGLangState()
    handler = _make_handler(state)
    httpd = ThreadingHTTPServer((host, port), handler)
    bound_port = httpd.server_address[1]
    server = MockSGLangServer(host=host, port=bound_port, state=state, _httpd=httpd)
    server._thread = threading.Thread(
        target=httpd.serve_forever,
        name="mock-sglang-http",
        daemon=True,
    )
    server._thread.start()
    return server


def send_dummy_generate_requests(
    base_url: str,
    *,
    count: int = 5,
    max_new_tokens: int = 32,
    latency_seconds: float = 0.05,
) -> list[dict]:
    """Send dummy ``POST /generate`` requests to a SGLang-compatible server."""

    import urllib.error
    import urllib.request

    results: list[dict] = []
    endpoint = f"{base_url.rstrip('/')}/generate"
    for index in range(count):
        payload = json.dumps(
            {
                "text": f"dummy prompt {index}",
                "max_new_tokens": max_new_tokens,
                "mock_latency_seconds": latency_seconds,
            }
        ).encode("utf-8")
        request = urllib.request.Request(
            endpoint,
            data=payload,
            headers={"Content-Type": "application/json"},
            method="POST",
        )
        try:
            with urllib.request.urlopen(request, timeout=10) as response:
                body = response.read().decode("utf-8")
                results.append(json.loads(body))
        except urllib.error.URLError as exc:
            results.append({"error": str(exc)})
    return results

# Probing HTTP API

## Routing model

| Layer | URL pattern | Registration |
|-------|-------------|--------------|
| **Server public** | `/apis/{resource}` | `server/api/mod.rs` (explicit Axum routes) |
| **Extension** | `/apis/{ext.name()}/{local_path}` | `@ext_handler` (Python) or `EngineCall` (Rust) |
| **SQL** | `POST /query` | DataFusion engine (not REST) |

Extension HTTP name comes from `EngineExtension::name()` (struct name lowercased, e.g. `PythonExt` → `pythonext`).

SQL namespace (`with_extension(..., namespace, ...)`) is separate and may differ (e.g. SQL `python.*`, HTTP `pythonext`).

## Server public API

Registered in `server/api/mod.rs`:

| Method | Path | Handler |
|--------|------|---------|
| GET | `/apis/overview` | System overview |
| GET | `/apis/files?path=…` | Read workspace file |
| GET/PUT | `/apis/nodes` | Cluster node list / register |
| POST | `/apis/nodes/sync` | Merge nodes from Pulsing |
| GET | `/apis/flamegraph/torch` | PyTorch CPU flamegraph (SVG) |
| GET | `/apis/flamegraph/pprof` | pprof flamegraph (SVG) |

## Extension API (`pythonext`)

All handlers live in `python/probing/handlers/pythonext.py`, one canonical local path each.

| Method | Path | Handler |
|--------|------|---------|
| GET | `/apis/pythonext/callstack?tid=&mode=` | `callstack` |
| POST | `/apis/pythonext/eval` | `eval` (body = code) |
| GET | `/apis/pythonext/trace/list` | `trace/list` |
| GET | `/apis/pythonext/trace/show` | `trace/show` |
| GET | `/apis/pythonext/trace/start` | `trace/start` |
| GET | `/apis/pythonext/trace/stop` | `trace/stop` |
| GET | `/apis/pythonext/trace/variables` | `trace/variables` |
| GET | `/apis/pythonext/trace/chrome-tracing` | `trace/chrome-tracing` |
| GET | `/apis/pythonext/pytorch/timeline` | `pytorch/timeline` |
| GET | `/apis/pythonext/pytorch/profile` | `pytorch/profile` |
| GET | `/apis/pythonext/ray/timeline` | `ray/timeline` |
| GET | `/apis/pythonext/ray/timeline/chrome` | `ray/timeline/chrome` |
| GET | `/apis/pythonext/magics` | `magics` |

Rust-backed endpoints (`callstack`, `eval`) are thin `@ext_handler` wrappers around `probing._core.api_callstack` / `api_eval`.

## Other extensions

| Extension | Example path | Notes |
|-----------|--------------|-------|
| `rdmaextension` | `POST /apis/rdmaextension/` | Rust `EngineCall`, CLI only |

## Top-level (non `/apis`)

| Method | Path | Purpose |
|--------|------|---------|
| POST | `/query` | SQL (`Message<Query>` JSON) |
| POST | `/query/dto` | SQL (JSON DTO, external clients) |
| GET | `/config/{config_key}` | Read config value |
| GET | `/ws` | WebSocket REPL |

## Adding endpoints

```
New HTTP endpoint?
├─ Stable platform / special HTTP semantics → server/api/mod.rs
└─ Extension-specific → @ext_handler("pythonext", "group/action") only
```

Do not register the same capability in both places. Do not add path aliases.

## HTTP status codes

| Case | Status |
|------|--------|
| Extension path in spec, wrong HTTP method | 405 |
| EEM / extension not found | 404 |
| Python handler JSON `{"error":"No handler found…"}` | 404 |
| Other Python handler JSON `{"error":…}` | 400 |
| Invalid query string on extension URL | 400 |
| Missing config key | 404 |
| Invalid `/query` JSON body | 400 |
| SET statement failure on `/query` | 500 (payload `QueryDataFormat::Error`) |
| Invalid file path / missing param | 400 |
| File too large | 413 |

## Extension response headers

Extension fallback responses (`server/api/extension.rs`) take `Content-Type` and CORS
from [`tests/spec/api_spec.json`](../../tests/spec/api_spec.json), not path substring
heuristics. Each handler declares:

```json
"response": { "content_type": "application/json", "cors": true }
```

Defaults live in `extension_response_defaults`. Lookup is implemented in
`server/api/response.rs` (compile-time embedded spec).

| Field | Meaning |
|-------|---------|
| `content_type` | `application/json` or `text/plain` |
| `cors` | When `true`, add CORS headers (timeline endpoints for Perfetto UI) |

When adding a pythonext handler, update the spec `response` block alongside
`pythonext_handlers` and `@ext_handler`.

## Client contracts (Web UI + CLI)

Web and CLI do **not** import Server routes. They share the same machine-readable
contract: [`tests/spec/api_spec.json`](../../tests/spec/api_spec.json), section
`client_contracts`.

Each entry lists the Rust source file and the HTTP calls it makes (`method` +
`path`). Contract tests in `tests/spec/client_contract.py` verify:

- declared paths exist in the canonical endpoint list (`server_public`,
  `pythonext_handlers`, `other_extensions`, `top_level`)
- path literals in source match the contract
- no deprecated paths (e.g. `/apis/python/…`) appear in client code

When adding or changing a Web/CLI HTTP call, update `client_contracts` in the
spec — not Server source.

```bash
uv run pytest tests/spec/test_api_spec.py -q
```

## Contract spec (machine-readable)

The canonical contract is [`tests/spec/api_spec.json`](../../tests/spec/api_spec.json).
Run contract tests:

```bash
uv run pytest tests/spec/test_api_spec.py -q
cargo test -p probing-server --no-default-features spec
cargo test -p probing-core --test extension_routing_spec
```

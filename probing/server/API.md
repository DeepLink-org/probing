# Probing HTTP API

The local Unix-socket server authenticates clients with operating-system peer
credentials and accepts only connections whose effective UID matches the training
process. Local CLI access requires no token. Remote TCP authentication is documented
in [AUTH.md](AUTH.md).

## Routing model

| Layer | URL pattern | Registration |
|-------|-------------|--------------|
| **Server public** | `/apis/{resource}` | `server/api/mod.rs` (explicit Axum routes) |
| **Extension** | `/apis/{ext.name()}/{local_path}` | `ProbeExtensionCall::routes` typed registration + `call` execution |
| **SQL** | `POST /query` | DataFusion engine (not REST) |

Extension HTTP name comes from `ProbeExtension::name()` (derived: struct lowercased, with `probeextension` → `extension`, e.g. `RdmaProbeExtension` → `rdmaextension`, `PythonExt` → `pythonext`).
Route registration fails engine initialization on duplicate/invalid paths; method,
content type, CORS, and engine-readiness requirements are part of `ExtensionRoute`.

SQL catalog registration uses [`EngineBuilder::with_data_source`](super::engine::EngineBuilder::with_data_source) and is separate from extension HTTP/SET wiring.

## Server public API

Registered in `server/api/mod.rs`:

| Method | Path | Handler |
|--------|------|---------|
| GET | `/apis/overview` | System overview |
| GET | `/apis/files?path=…` | Read workspace file |
| GET/PUT | `/apis/nodes` | Cluster node list / register |
| GET | `/apis/training/step_matrix` | Cross-rank train.step samples (`cluster=false` default; set `cluster=true` for on-demand fan-out) |
| POST | `/apis/cluster/query` | On-demand SQL fan-out (`{"expr":"…","cluster":true}`; read-only SQL only) |

Flamegraphs are served by profiler extensions (extension fallback, not public routes):

| Method | Path | Notes |
|--------|------|-------|
| GET | `/apis/torchextension/flamegraph` | PyTorch module flamegraph (interactive HTML) |
| GET | `/apis/torchextension/flamegraph/json` | JSON for native Web UI (`?metric=` optional) |
| GET | `/apis/torchextension/flamegraph/distributed/json` | SPMD torch module flamegraph at one `local_step` (`?cluster=true` default, `?step=`, `?metric=`). Requires the query engine to be ready. |
| GET | `/apis/pprofextension/flamegraph` | CPU sampling flamegraph (interactive HTML) |
| GET | `/apis/pprofextension/flamegraph/json` | JSON for native Web UI |
| GET | `/apis/pprofextension/flamegraph/folded/json` | Raw folded stack lines for cluster merge |
| GET | `/apis/pprofextension/flamegraph/distributed/json` | Distributed SIGPROF stack flamegraph (`?cluster=true` default, `?mode=mixed\|py`). Peer capture is bounded-concurrent with an overall deadline; completed peers are returned on timeout with `nodesFailed`. Frames may include `ranks: [i32]` and payload `rankCount`. |

Removed public aliases (see `api_spec.json` `deprecated_paths`): `/apis/training/distributed_flamegraph/json`, `/apis/training/distributed_stack_flamegraph/json`.

## Cluster query (on-demand fan-out)

Training agents write to **local memtable only**. Cross-node aggregation is explicit:

- **Local** (default): `GET /apis/training/step_matrix?cluster=false` or `POST /apis/cluster/query` with `"cluster": false`
- **Cluster scan**: `cluster=true` fans out the same SQL to peer nodes from the in-memory cluster view (torchrun report / `PUT /apis/nodes`), merges rows, and tags `_host` / `_addr` (plus `_rank`, `_node_rank`, `_local_rank`, `_role` on federated results)

CLI:

```bash
probing -t host:8080 cluster query "SELECT rank, local_step, duration_ms FROM python.comm_collective LIMIT 20"
probing -t host:8080 cluster query --local "SELECT * FROM python.comm_collective LIMIT 5"
probing -t host:8080 cluster nodes
```

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
| GET | `/apis/pythonext/pytorch/profile` | `pytorch/profile` — start profiler (legacy alias of `profile/start`; prefer `/start`) |
| GET | `/apis/pythonext/pytorch/profile/start` | `pytorch/profile/start` — `steps`, `trigger` (canonical; Web clients use this) |
| GET | `/apis/pythonext/pytorch/profile/stop` | `pytorch/profile/stop` — finalize capture |
| GET | `/apis/pythonext/pytorch/profile/status` | `pytorch/profile/status` |
| GET | `/apis/pythonext/pytorch/runtime-debug?include_values=` | `pytorch/runtime-debug` — local wait counters + read-only job TCPStore snapshot |
| GET | `/apis/pythonext/ray/timeline` | `ray/timeline` |
| GET | `/apis/pythonext/ray/timeline/chrome` | `ray/timeline/chrome` |
| GET | `/apis/pythonext/magics` | `magics` |
| GET | `/apis/pythonext/skills/list` | `skills/list` — merged catalog |
| GET | `/apis/pythonext/skills/load?id=` | `skills/load` — one skill JSON |
| GET | `/apis/pythonext/skills/catalog` | `skills/catalog` |
| GET | `/apis/pythonext/skills/routing` | `skills/routing` — catalog + intents + pages |
| GET | `/apis/pythonext/skills/roots` | `skills/roots` — discovered skill directories |
| GET | `/apis/pythonext/extensions/list` | `extensions/list` — installed `probing-<vendor>` packages |
| GET | `/apis/pythonext/flight-recorder/snapshot?include_stack_traces=&only_active=&persist=` | `flight-recorder/snapshot` |

`pytorch/runtime-debug` reports wait counters for the connected rank only; the
TCPStore catalog represents rendezvous state shared by the job. Unknown value
payloads stay redacted unless the server has `PROBING_TCPSTORE_INSPECT=1` and the
request explicitly sets `include_values=true`. Runtimes without TCPStore key
enumeration report `total_keys` with `catalog_available: false`. The wait-counter
snapshot includes a `source`: `pytorch` identifies the native experimental
PyTorch handler; an explicitly registered compatibility provider reports its own
source label instead.

Skill HTTP endpoints above are **discovery only** (catalog, routing, load JSON). Execution
uses the Rust `probing-skills` runner: CLI `probing skill run`, MCP `run_skill` /
`plan_skill`, or Web Investigate Agent (WASM).

Rust-backed endpoints (`callstack`, `eval`) are thin `@ext_handler` wrappers around `probing._core.api_callstack` / `api_eval`.

## Other extensions

| Extension | Example path | Notes |
|-----------|--------------|-------|
| `torchextension` | `GET /apis/torchextension/flamegraph` | Rust `ProbeExtensionCall`; torch module flamegraph |
| `torchextension` | `GET /apis/torchextension/flamegraph/json` | Torch flamegraph JSON (`?metric=` optional) |
| `pprofextension` | `GET /apis/pprofextension/flamegraph` | CPU SIGPROF flamegraph HTML |
| `pprofextension` | `GET /apis/pprofextension/flamegraph/json` | pprof flamegraph JSON |
| `rdmaextension` | `POST /apis/rdmaextension/` | Rust `ProbeExtensionCall`, CLI only |

## Top-level (non `/apis`)

| Method | Path | Purpose |
|--------|------|---------|
| POST | `/query` | SQL (`Message<Query>` JSON) |
| POST | `/query/dto` | SQL (JSON DTO, external clients) |

Core SQL execution returns `QueryOutcome<DataFrame>` (`data` + `quality`). The
common `/query` envelope serializes non-default quality as typed
`meta.fanout` (`nodes_succeeded`, `nodes_failed`, `peer_batches_dropped`, `partial`).
| GET | `/config/{config_key}` | Read config value |
| GET | `/ws` | WebSocket REPL |
| * | `/mcp` | MCP Streamable HTTP (agent tools + schema resources) |

### MCP (Model Context Protocol)

When built with the `rmcp` feature (default in the PyPI wheel), the server exposes MCP at **`/mcp`** using [Streamable HTTP](https://modelcontextprotocol.io/specification/2025-03-26/basic/transports#streamable-http).

#### Read tools (always enabled)

| Tool | Purpose |
|------|---------|
| `query` | Read-only SQL against the in-process engine (`limit` caps rows; rejects multi-statement batches containing writes) |
| `describe_tables` | Semantic docs from `probe.probing.table_docs` / `column_docs` |
| `list_skills` | List diagnostic skills (bundled + entry-point extensions) |
| `plan_skill` | Expand a skill into SQL/API steps (Rust in-process) |
| `run_skill` | Execute a diagnostic skill (Rust in-process) |
| `list_cluster_nodes` | `GET /apis/nodes` — registered cluster members |
| `cluster_query` | Read-only SQL with optional cluster fan-out (`cluster` default `true`; AST-validated, same rules as `query`) |

#### Write tools (disabled unless `PROBING_MCP_ALLOW_WRITE=1`)

| Tool | Purpose |
|------|---------|
| `set_config` | `SET probing.* = …` runtime config |
| `eval_python` | `POST /apis/pythonext/eval` — run Python in the training process |

Write calls are logged at `info` as `MCP write audit: …`.

#### Resources

| URI | Content |
|-----|---------|
| `probing://schema/catalog` | All table docs (JSON) |
| `probing://schema/{schema}/{table}` | One table doc + column docs (JSON) |

Cursor / Claude Code example (remote training agent on `host:8080`):

```json
{
  "mcpServers": {
    "probing": {
      "url": "http://127.0.0.1:8080/mcp"
    }
  }
}
```

Enable intervention tools for an on-call agent:

```bash
export PROBING_MCP_ALLOW_WRITE=1
```

For the in-process unix-socket server (same PID as training), point MCP at the TCP address from `server.address` config after `PROBING=1` startup.

## Adding endpoints

```
New HTTP endpoint?
├─ Stable platform / special HTTP semantics → server/api/mod.rs
└─ Extension-specific → @ext_handler("pythonext", "group/action") only
```

Do not register the same capability in both places. Do not add path aliases.

## HTTP status codes

Control-plane API failures use one JSON envelope so Web and CLI clients can preserve
the real cause instead of replacing it with the HTTP reason phrase:

```json
{
  "error": {
    "code": "SERVICE_UNAVAILABLE",
    "message": "probing engine is still starting",
    "retryable": true,
    "action": "retry after /ready reports ready"
  }
}
```

`action` is optional. Query endpoints retain their versioned query DTO/message payloads,
extension handlers retain their documented native response body, and `/health` plus
`/ready` retain their orchestrator-specific responses. Clients accept both the canonical
envelope and these documented endpoint-specific formats.

| Case | Status |
|------|--------|
| Extension path in spec, wrong HTTP method | 405 |
| EEM / extension not found | 404 |
| Python handler JSON `{"error":"No handler found…"}` | 404 |
| Other Python handler JSON `{"error":…}` | 400 |
| Invalid query string on extension URL | 400 |
| Missing config key | 404 |
| Invalid `/query` JSON body | 400 |
| SQL/config execution failure on `/query` | 500 (`QueryDataFormat::Error` payload preserved) |
| `/query/dto` engine errors | Same HTTP status as underlying `ApiError` (e.g. 404, 503); DTO `code` mirrors status (`BAD_REQUEST`, `NOT_FOUND`, `SERVICE_UNAVAILABLE`, …) |
| Partial cluster fan-out (`meta.fanout.partial` / `nodes_failed` non-empty) on `/query`, `/query/dto`, `POST /apis/cluster/query`, `GET /apis/training/step_matrix`, or a distributed Torch/pprof flamegraph | 503 (body still returned so clients can inspect partial data) |
| Invalid extension query parameter (`cluster`, `step`, `metric`, or `mode`) | 400 |
| Invalid file path / missing param | 400 |
| File too large | 413 |

## Extension response headers

Extension fallback responses (`server/api/extension.rs`) take `Content-Type` and CORS
from the registered typed `ExtensionRoute`, not path substring heuristics or a
test artifact embedded in production. Regression tests compare those registered
contracts with [`api_spec.json`](../../tests/regression/spec/api_spec.json).

| Field | Meaning |
|-------|---------|
| `content_type` | `application/json` or `text/plain` |
| `cors` | When `true`, add CORS headers (timeline endpoints for Perfetto UI) |

When adding a pythonext handler, update the spec `response` block alongside
`pythonext_handlers` and `@ext_handler`.

## Client contracts (Web UI + CLI)

Web and CLI do **not** import Server routes. They share the same machine-readable
contract: [`tests/regression/spec/api_spec.json`](../../tests/regression/spec/api_spec.json), section
`client_contracts`.

Each entry lists the Rust source file and the HTTP calls it makes (`method` +
`path`). Contract tests in `tests/regression/spec/client_contract.py` verify:

- declared paths exist in the canonical endpoint list (`server_public`,
  `pythonext_handlers`, `other_extensions`, `top_level`)
- path literals in source match the contract
- no deprecated paths (e.g. `/apis/python/…`) appear in client code

When adding or changing a Web/CLI HTTP call, update `client_contracts` in the
spec — not Server source.

```bash
uv run pytest tests/regression/spec/test_api_spec.py -q
```

## Contract spec (machine-readable)

The canonical contract is [`tests/regression/spec/api_spec.json`](../../tests/regression/spec/api_spec.json).
Run contract tests:

```bash
uv run pytest tests/regression/spec/test_api_spec.py -q
cargo test -p probing-rust-regression server_training_observability --no-default-features
```

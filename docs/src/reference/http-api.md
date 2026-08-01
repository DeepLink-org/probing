# HTTP & MCP API

This page maps the public surfaces to their canonical contracts. It intentionally avoids
duplicating every endpoint field.

## Surface map

| Surface | Entry point | Contract |
|---------|-------------|----------|
| SQL | `POST /query`, `POST /query/dto` | `probing-proto` DTOs and API spec |
| Server resources | `/apis/*` | Explicit Axum routes |
| Extension resources | `/apis/<extension>/*` | `ProbeExtension` / `@ext_handler` |
| Configuration | `/config/{key}` and SQL `SET` | Extension options |
| WebSocket REPL | `GET /ws` | CLI REPL client |
| MCP | `/mcp` | Streamable HTTP tools and resources |
| Health | `GET /health`, `GET /ready` | Liveness and engine readiness JSON |

The human-readable endpoint inventory, authentication forms, response headers, and status
codes are maintained in
[`probing/server/API.md`](https://github.com/DeepLink-org/probing/blob/main/probing/server/API.md).
The machine-readable source of truth is
[`tests/regression/spec/api_spec.json`](https://github.com/DeepLink-org/probing/blob/main/tests/regression/spec/api_spec.json),
enforced by contract tests.

## Authentication

Local Unix connections use same-effective-UID peer credentials. TCP routes are protected only
when `PROBING_AUTH_TOKEN` is non-empty; health and static paths remain public. See
[Security](../operations/security.md) before enabling remote access.

## MCP capability model

Read tools include `query`, `describe_tables`, skill planning/execution, node listing, and
cluster query. `set_config` and `eval_python` remain unavailable until
`PROBING_MCP_ALLOW_WRITE=1`.

Schema resources:

| URI | Content |
|-----|---------|
| `probing://schema/catalog` | All registered table and column documentation |
| `probing://schema/{schema}/{table}` | Documentation for one table |

## Error and partial-result semantics

- Invalid requests use HTTP `4xx`; unavailable engine and incomplete cluster results use
  `503` where specified by the route contract.
- Partial cluster bodies are retained so clients can inspect failed nodes and dropped peer
  batches. Consumers must check both HTTP status and response metadata.
- Federation broadcast queries require a statically bounded top-level `LIMIT` by default, and
  coordinator materialization is capped by `PROBING_GLOBAL_SCAN_MAX_ROWS`.
- Peer/final response bodies are capped by `PROBING_GLOBAL_RESPONSE_MAX_BYTES`; cumulative
  protocol/Arrow materialization is capped by `PROBING_GLOBAL_MEMORY_MAX_BYTES`. Budget exhaustion
  uses `503`; remote exhaustion can appear as an explicitly marked partial result.
- Hierarchical fan-out propagates the effective limits in `X-Probing-Response-Max-Bytes` and
  `X-Probing-Memory-Max-Bytes`. Receivers only allow these headers to narrow local limits.

CLI and Python signatures are separate from the wire protocol; see
[CLI & Python API](../api-reference.md).

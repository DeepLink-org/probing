# Production deployment

Probing runs inside the observed Python process. Deployment choices therefore affect the
availability and attack surface of the training process itself.

## Local-only baseline

Prefer PID targeting when the CLI and target share a host and effective UID:

```bash
PROBING=1 python train.py
probing -t <pid> query "SHOW TABLES"
```

This path uses the Unix control socket and does not require a token. It does not create a
remote TCP trust boundary.

## Remote TCP baseline

Create the token outside the command history, deliver it through the workload's secret
mechanism, and expose the listener only through a trusted network or TLS proxy:

```bash
export PROBING_SERVER_ADDR="'127.0.0.1:8080'"
export PROBING_AUTH_TOKEN="<random-secret>"
PROBING=1 python train.py
```

The inner quotes make the value a string literal for runtime configuration. Alternatively,
`PROBING_PORT=8080` enables a listener on all interfaces. The CLI reads the
same `PROBING_AUTH_TOKEN` and sends a Bearer credential. Binding to
`0.0.0.0` without a token leaves non-public routes unauthenticated; see
[Security](security.md).

## Health and startup

Configure liveness against `GET /health` and readiness against `GET /ready`. Keep them
separate: restarting a live process solely because the SQL engine is still initializing can
interrupt the training workload.

Engine initialization failures leave the HTTP server available and `/ready` returns `503` by
default. Set `PROBING_ENGINE_FAIL_FAST=1` only when loss of diagnostics should terminate the
target workload.

## Resource boundaries

| Boundary | Setting | Default / behavior |
|----------|---------|--------------------|
| Request body | `PROBING_MAX_REQUEST_SIZE` | 5 MiB |
| Concurrent HTTP connections | `PROBING_MAX_CONNECTIONS` | max of fan-out concurrency and 128 |
| Federated rows | `PROBING_GLOBAL_SCAN_MAX_ROWS` | 10,000 |
| Federated peer/response bytes | `PROBING_GLOBAL_RESPONSE_MAX_BYTES` | 16 MiB |
| Federated materialization | `PROBING_GLOBAL_MEMORY_MAX_BYTES` | 128 MiB cumulative per query |
| Peer query timeout | `PROBING_REMOTE_QUERY_TIMEOUT_SECS` | 30 seconds |
| Fan-out concurrency | `PROBING_FANOUT_CONCURRENCY` | 128, further clamped by byte budgets |
| Python table memory | `PROBING_TABLE_DEFAULT_MB` | 20 MiB per table unless overridden |

These are guardrails, not a memory quota for the whole process. Query operators, collector count,
and cold storage add separate costs.

## Data lifecycle

MEMT is a bounded mmap ring: new writes eventually reuse old chunks. Optional MEMC cold
compaction extends retention but is still diagnostic storage, not a backup system. Configure
`PROBING_DATA_DIR`, cold-store size, age, and TTL explicitly when retention matters. See
[Data Layer](../design/data-layer.md) for the format and write semantics.

## Deployment checklist

- Use the Unix socket for same-host access where possible.
- For TCP, set a non-empty token and terminate TLS before the Probing listener.
- Restrict the bind address, firewall rules, and `PROBING_ALLOWED_FILE_DIRS`.
- Leave MCP write tools disabled unless intervention is required.
- Probe `/health` and `/ready` independently.
- Set request, connection, federation, and storage limits for the workload size.
- Decide whether partial federation is acceptable; otherwise set `PROBING_FANOUT_STRICT=1`.
- Test upgrade and rollback against a disposable training job before changing a live fleet.

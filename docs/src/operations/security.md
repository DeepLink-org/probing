# Security

Probing can read process state, query telemetry, read configured files, and execute Python
when the corresponding endpoint is authorized. Treat its control endpoint as privileged
access to the training process.

## Trust boundaries

| Surface | Default authorization | Transport security | Important consequence |
|---------|-----------------------|--------------------|-----------------------|
| Local Unix socket | Effective UID must match the target process | Local kernel IPC | Another process under the same UID is trusted |
| Remote TCP | No authentication until `PROBING_AUTH_TOKEN` is non-empty | Plain HTTP | Do not expose an unconfigured listener |
| MCP read tools | Available to an authenticated connection | Same as endpoint | SQL and diagnostic metadata are readable |
| MCP write tools | Disabled unless `PROBING_MCP_ALLOW_WRITE=1` | Same as endpoint | Enables config mutation and Python evaluation |
| HTTP eval/config routes | Protected by TCP auth when configured | Same as endpoint | The MCP write flag does not disable direct HTTP capabilities |

## Local mode

The Unix listener reads operating-system peer credentials before HTTP handling and rejects a
client whose effective UID differs from the server's effective UID. There is no caller-
supplied UID header or token to forge.

This boundary assumes that processes running under the same account trust each other. Use
separate operating-system identities or stronger sandboxing for mutually untrusted jobs.

## Remote TCP mode

Set a high-entropy token before the server starts:

```bash
export PROBING_AUTH_TOKEN="<random-secret>"
export PROBING_SERVER_ADDR="'127.0.0.1:8080'"
```

Clients may send `Authorization: Bearer <token>`, HTTP Basic authentication with the token as
the password, or `X-Probing-Token`. Prefer Bearer authentication for programmatic clients.
The CLI reads `PROBING_AUTH_TOKEN` automatically.

An unset or empty token disables TCP authentication. Probing does not terminate TLS; use a
reverse proxy, service mesh, SSH tunnel, or equivalent encrypted channel. Limit network reach
even when a token is configured.

## Public paths

The TCP authentication middleware intentionally leaves these paths public:

- `/health` and `/ready`
- `/`, `/index.html`, `/static/*`, and favicon paths

Do not place secrets or workload details in public static assets or health responses.

## Capability controls

- Keep `PROBING_MCP_ALLOW_WRITE` unset for diagnostic-only agents.
- Enabling MCP writes permits `set_config` and `eval_python`; write calls are audit-logged.
- Direct HTTP/CLI eval is a separate capability. Network authentication controls who can call
  it, but the MCP flag is not a global eval kill switch.
- Restrict `PROBING_ALLOWED_FILE_DIRS`; the file API also includes built-in allowed roots.
- `GET /apis/overview` filters token- and secret-shaped environment keys, but endpoint access
  should still be treated as sensitive.

## Token operations

- Deliver tokens through the scheduler or secret manager, not source control or command-line
  arguments.
- Use a distinct token per trust domain and rotate it after suspected disclosure.
- Changing `server.auth_token` at runtime changes the middleware credential; coordinate client
  rollout to avoid losing access.
- Never put a token in a URL query string, logs, screenshots, or diagnostic bundles.

## Security checklist

- [ ] Local access is sufficient, or TCP exposure is explicitly required.
- [ ] Every non-loopback TCP listener has a non-empty token.
- [ ] TLS and network policy protect the TCP hop.
- [ ] File roots and MCP write access follow least privilege.
- [ ] Same-UID process trust is acceptable for the host.
- [ ] Public health/static paths contain no sensitive information.
- [ ] Token rotation and incident revocation have an owner.

Endpoint details and status codes: [HTTP & MCP API](../reference/http-api.md). Exact variables:
[Environment Variables](../reference/env-vars.md).

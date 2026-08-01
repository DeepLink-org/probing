# Start here

This section establishes one working connection to a probed process before introducing
the SQL model or distributed execution.

## Choose a connection mode

| Mode | Use | Security boundary | First command |
|------|-----|-------------------|---------------|
| In-process, local | Start the target with Probing enabled | Unix peer credentials; same effective UID | `PROBING=1 python train.py` |
| Inject, local | Attach to an existing Linux process | Unix peer credentials; same effective UID | `probing -t <pid> inject` |
| Remote TCP | Query across a host or cluster | Token when configured; deploy behind TLS | `probing -t <host>:<port> query …` |

For TCP exposure, complete the [security checklist](../operations/security.md) before
binding beyond loopback.

## Recommended order

1. [Installation](../installation.md) — install the wheel and check platform support.
2. [Quick Start](../quickstart.md) — connect, capture a backtrace, and run a SQL query.
3. [Core model](../guide/concepts.md) — understand catalogs, endpoints, and step coordinates.
4. [Operations](../operations/index.md) — configure health checks, limits, and authentication.

## Completion check

The setup is working when the target answers a bounded query:

```bash
probing -t <pid-or-host:port> query "SHOW TABLES"
```

Continue with [SQL Analytics](../guide/sql-analytics.md) for query patterns or
[Diagnostic Skills](../guide/skills.md) for predefined diagnostic workflows.

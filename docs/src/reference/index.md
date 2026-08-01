# Reference

Authoritative lookup for SQL schemas, commands, wire protocols, and runtime configuration.

| Page | Contents |
|------|----------|
| **[SQL Tables](sql-tables.md)** | Physical columns for `python.*`, `cluster.*`, and federation tags (synced with `tables.yaml`) |
| **[CLI & Python API](../api-reference.md)** | CLI, in-process Python API, config, [unimplemented APIs](../api-reference.md#unimplemented-apis) |
| **[HTTP & MCP API](http-api.md)** | Wire surfaces, authentication boundary, canonical endpoint and machine-readable contracts |
| **[Environment Variables](env-vars.md)** | Supported user-configurable `PROBING_*` variables, defaults, and subsystem ownership |
| **[Skill Format](skill-format.md)** | `steps.yaml` and `SKILL.md` specification for diagnostic skill authors |
| **[Versions](../versions.md)** | Release compatibility and upgrade notes |

## Contract ownership

| Contract | Source of truth | Validation |
|----------|-----------------|------------|
| HTTP routes and client calls | `tests/regression/spec/api_spec.json` | `pytest tests/regression/spec/ -q` |
| SQL table semantics | Collector schemas and table docs | SQL table reference / regression tests |
| Environment variables | Reading implementation in the owning module | Documentation review and strict site build |
| Skills | `skills/<id>/SKILL.md` + `steps.yaml` | `python -m probing.skills validate` |

For narrative guides, see **[User Guide](../guide/index.md)**; for deployment decisions, see
**[Operations](../operations/index.md)**.

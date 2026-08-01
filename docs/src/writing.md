# Documentation style

Probing docs follow the tone of infrastructure projects (Kubernetes, DataFusion, Tokio):
**spec-first, mechanism-first**. Describe what the system is and how it behaves; avoid
product positioning and scenario marketing.

## Prefer

| Style | Example |
|-------|---------|
| **Logical** — structure, dependencies, invariants | «Federation rewrites `probe.*` → `global.*` at the coordinator; peers never recurse.» |
| **Technical** — APIs, data paths, defaults, failure modes | «`POST /apis/cluster/query` defaults `hierarchical=true`; missing `local_rank` falls back to flat fan-out.» |
| **Neutral headings** | «Execution model», «Catalog rewrite», «Example SQL» |
| **Tables for contracts** | Entry points, env vars, tag columns, path A/B/C conditions |

## Avoid

| Style | Example (do not write) |
|-------|------------------------|
| **Functional / marketing** | «Debug hanging jobs without reproducing», «essential for tail latency» |
| **Strategic / mission** | «Product goal», «Probing's mission is to make distributed Pythonic» |
| **Persona routing** | «I want to debug…», «Read when you…» |
| **Outcome promises** | «Find the exact module that's blocking», «90% of diagnostics» |
| **Diagnostic story arcs** | «Straggler chain: rank → machine → heatmap» as narrative; use query-pattern headings instead |

## Document roles

| Area | Audience | Content |
|------|----------|---------|
| **Reference** | Lookup | Schemas, CLI flags, env vars, HTTP DTOs — no tutorials |
| **Guide** | Operators | Commands + SQL that exercise documented behavior |
| **Architecture** | Contributors | Layers, crates, protocols, algorithms, regression queries |
| **Examples** | End-to-end | Reproducible commands against sample workloads |
| **Operations** | Deployment owners | Trust boundaries, health, limits, failure handling, runbooks |

## Information architecture

Every page belongs to one primary section:

| Question answered | Section | Required shape |
|-------------------|---------|----------------|
| How do I establish a working connection? | Getting Started | Ordered steps with a verifiable completion check |
| How do I perform a task? | Guide | Prerequisites, command/query, interpretation, next step |
| How do I run this safely and reliably? | Operations | Defaults, limits, failure semantics, checklist |
| How does the implementation work? | Architecture | Ownership, data flow, invariants, known limits, tests |
| What is the exact contract? | Reference | Names, types, defaults, errors; no narrative duplication |
| Can I reproduce a complete workflow? | Examples | Inputs, commands, expected evidence, cleanup |

Do not put implementation design into a guide or operational advice into a reference table.
Cross-link to the owning page instead.

## Sources of truth

| Content | Canonical source |
|---------|------------------|
| HTTP endpoints and client calls | `tests/regression/spec/api_spec.json`; inventory in `probing/server/API.md` |
| Table and column semantics | Collector schema/table docs; published index in `reference/sql-tables.md` |
| CLI syntax | Clap definitions; summarized in `api-reference.md` |
| Environment defaults | The module that reads the variable; summarized in `reference/env-vars.md` |
| Architecture boundaries | `design/modularity.md` |
| Diagnostic workflow | `skills/<id>/SKILL.md` + `steps.yaml` |

A summary may point to a canonical source but must not silently redefine it.

## Bilingual

English and Chinese pages should share the same **section structure and contracts**.
Translate mechanism, not slogans. If only one language is complete, the stub links to the
other without marketing filler.

- New navigation entries require both `.md` and `.zh.md` pages, or an explicit fallback stub.
- Keep headings and contract tables structurally aligned so language switching preserves place.
- Update both languages in one change when a default, guarantee, or security boundary changes.

## Cross-links

- Architecture defers usage to Guide; Guide defers contracts to Reference and Architecture.
- One SSOT per topic (`modularity.md` for layers, `federation.md` for cross-rank SQL semantics).

## Page quality checklist

- The first paragraph states scope and links to prerequisite material.
- Commands are copyable and use placeholders such as `<pid>` consistently.
- Defaults, platform constraints, failure/partial-result behavior, and security impact are explicit.
- Claims describe implemented behavior; drafts and proposals carry a visible status.
- A user-visible change updates Guide/Operations and the relevant Reference contract.
- English and Chinese structures remain aligned.
- Relative links resolve under `docs/src`; external repository links use permanent file paths.
- `make docs` succeeds with MkDocs strict mode before merge.

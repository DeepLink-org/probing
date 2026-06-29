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

## Bilingual

English and Chinese pages should share the same **section structure and contracts**.
Translate mechanism, not slogans. If only one language is complete, the stub links to the
other without marketing filler.

## Cross-links

- Architecture defers usage to Guide; Guide defers contracts to Reference and Architecture.
- One SSOT per topic (`modularity.md` for layers, `federation.md` for cross-rank SQL semantics).

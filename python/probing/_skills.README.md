# Bundled diagnostic skills (wheel copy)

**Do not edit skill content here.** This tree is generated from repo-root
[`skills/`](../../../skills/README.md):

```bash
make wheel         # bundles skills/ and web/dist/ automatically
```

Shipped inside the PyPI wheel so `probing skill list` and install tooling work
without a git checkout. At runtime, repo / user / project skill dirs can override
these entries (see [`../skills/README.md`](../skills/README.md)).

Python loader code lives in [`../skills/`](../skills/README.md) (`probing.skills` package).

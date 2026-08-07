# Bundled diagnostic skills

Skill **data** lives in [`bundled_skills/`](bundled_skills/README.md) (SSOT, packaged in the wheel).
Repo-root [`skills/`](../../../skills) is a **symlink** to `bundled_skills/` for authoring ergonomics
(`./skills/install.sh`, docs, L4 layout).

```bash
# Edit either path — they are the same directory:
ls skills/catalog.yaml
ls python/probing/bundled_skills/catalog.yaml
```

Python loader / install code lives in [`skills/`](skills/README.md) (`probing.skills` package) —
that is a different folder from `bundled_skills/`.

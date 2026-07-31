# Probing test layout

Two layers: **unit** (mirrors `python/probing/`) and **regression** (integration, contract, E2E).

```text
tests/
├── conftest.py                 # shared: sys.path, faulthandler
├── unit/
│   ├── conftest.py             # PROBING=0, no engine
│   └── probing/                # mirrors python/probing/
│       ├── skills/             # loader, interpret
│       ├── handlers/           # router, pythonext handlers
│       ├── hooks/              # import_hook
│       └── test_dev_pth.py
└── regression/
    ├── conftest.py             # PROBING=1, engine wait, collective reset
    ├── spec/                   # api_spec.json + contract tests
    ├── core/ ext/ repl/ tbls/ inspect/ profiling/
    ├── training_observability/
    ├── skills/ nccl/
    └── rust/                   # probing-rust-regression crate
```

## Rust

| Kind | Location | Run |
|------|----------|-----|
| **Unit** | `#[cfg(test)]` in `probing/**/src/` | `make test-rust-unit` |
| **Regression** | `tests/regression/rust/probing/**` | `make test-rust-regression` |

Long-running signal stability is kept out of pull-request CI. The
`Signal Stability Soak` workflow runs the `python-signal-soak` regression
binary for 30 minutes on native Linux x86_64 and aarch64 runners, exercising
SIGPROF/pprof and SIGUSR2 concurrently. A short Linux run:

```bash
PROBING_SIGNAL_SOAK_SECS=30 cargo run --release \
  -p probing-rust-regression --bin python-signal-soak
```

## Python

| Kind | Location | Run |
|------|----------|-----|
| **Unit** | `tests/unit/probing/...` | `make test-python-unit` |
| **Regression** | `tests/regression/...` | `make test-python-regression` |

Test data factories: `python/probing/testing/` (not a test directory).

## Markers

- `unit` / `regression` / `integration` / `slow` — see `pyproject.toml`

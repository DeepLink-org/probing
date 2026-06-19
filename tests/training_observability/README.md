# Training observability test suite

End-to-end and layered tests for **collective tracing**, **train.step spans**,
**3D topology context**, and **SQL analytics** used by the Training page / cluster
fan-out control plane.

## Layout

| Module | Layer | What it validates |
|--------|-------|-------------------|
| `test_collective_policy.py` | Policy | autostart, lite/full/trace_event config |
| `test_collective_recording.py` | Agent / storage | `comm_collective` + closed `trace_event` |
| `test_collective_tracer_hook.py` | Hook | `CollectiveTracer` with mocked `torch.distributed` |
| `test_topology_context.py` | Context | TP/PP/DP env → span & comm rows |
| `test_step_straggler_sql.py` | Analytics | train.step join semantics (memtable; mirrors server SQL) |
| `test_training_iteration_e2e.py` | E2E | one synthetic iteration + regressions |
| `test_training_sql_integration.py` | SQL | `comm_collective` via in-process engine (run last) |

Shared fixtures live in `conftest.py` (`rank_env`, `parallel_env`, `sql_query`).

## Run

```bash
# Full suite (needs maturin develop + PROBING=1)
PROBING=1 pytest tests/training_observability -q

# By marker
PROBING=1 pytest -m training_observability -q

# Rust fan-out unit tests
cargo test -p probing-server training_observability --no-default-features
```

## Relationship to `tests/ext/`

Older files under `tests/ext/test_comm_collective.py`, `test_collective_autostart.py`,
and `test_parallel_topology.py` remain as lightweight smoke tests. New coverage should
be added to this package first; ext tests can delegate or be trimmed once CI relies on
this suite.

## Server-side tests

Rust merge/fan-out logic: `probing/server/tests/training_observability_tests.rs` and
unit tests in `server/src/server/cluster_fanout.rs`.

HTTP integration tests for `GET /apis/training/step_matrix` and `POST /apis/cluster/query`
are planned once a lightweight in-process server fixture lands (auth + engine bootstrap).

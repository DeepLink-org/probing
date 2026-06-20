# Examples

Runnable scripts under `examples/`. They are **not** installed with `pip install probing` or `make develop`.

## Dependencies

| Script | Extra packages | Notes |
|--------|----------------|-------|
| `events.py`, `hooks.py`, `test_probing.py` | none (beyond probing) | Good smoke tests |
| `imagenet.py`, `imagenet_with_span.py` | `torch`, `torchvision` | Needs ImageNet data path |
| `ray_tracing_example.py` | `ray` | Optional Ray integration |
| `bench_profiler.py` | varies | See script header |

Install ML stack into your dev venv:

```bash
source .venv/bin/activate
uv pip install torch torchvision
# or: pip install torch torchvision
```

## Running with probing

Use the project venv after `make develop` (see [Contributing](../docs/src/contributing.md)):

```bash
source .venv/bin/activate
PROBING=1 python examples/events.py
PROBING=1 python examples/test_probing.py --depth 2
```

On Linux you can also attach with `probing -t <pid> inject` instead of `PROBING=1` at startup.

## More documentation

- [Examples (MkDocs)](../docs/src/examples/index.md)
- [Quick Start](../docs/src/quickstart.md)

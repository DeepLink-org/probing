#!/usr/bin/env bash
# Instrumentation 开销基准（span / phase / TorchProbe）。
#
#   ./examples/overhead/run_bench.sh
#   ./examples/overhead/run_bench.sh --quick
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"
[[ -f .venv/bin/activate ]] && source .venv/bin/activate
export PROBING="${PROBING:-1}"
exec python examples/overhead/bench_instrumentation.py "$@"

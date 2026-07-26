#!/usr/bin/env bash
# TorchProbe shadow-step 开销冒烟（无需 GPU）。
#
#   ./examples/overhead/run_torch_probe_smoke.sh
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"
[[ -f .venv/bin/activate ]] && source .venv/bin/activate
export PROBING="${PROBING:-1}"
exec python examples/overhead/torch_probe_overhead_smoke.py "$@"

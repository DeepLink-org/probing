#!/usr/bin/env bash
# Ray actor span demo（slime 风格 async RL 骨架）。
#
#   ./examples/ray/run_job_actor.sh
#   PROBING_PORT=8080 ./examples/ray/run_job_actor.sh
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"
[[ -f .venv/bin/activate ]] && source .venv/bin/activate
export PROBING="${PROBING:-1}"
export PROBING_PORT="${PROBING_PORT:-8080}"
exec python examples/ray/ray_job_actor_span_demo.py "$@"

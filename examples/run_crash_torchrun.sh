#!/usr/bin/env bash
# torchrun crash demo — run from repo root: ./examples/run_crash_torchrun.sh [mode] [crash-rank]
#   mode: record (default) | exception | all
set -euo pipefail

cd "$(dirname "$0")/.."
[[ -f .venv/bin/activate ]] && source .venv/bin/activate

MODE="${1:-record}"
CRASH_RANK="${2:-1}"
NPROC="${NPROC:-4}"
MASTER_PORT="${MASTER_PORT:-29571}"

export PROBING="${PROBING:-regex:crash_torchrun_demo.py}"
export MASTER_ADDR=127.0.0.1
export MASTER_PORT
[[ "$(uname -s)" == Darwin ]] && export GLOO_SOCKET_IFNAME="${GLOO_SOCKET_IFNAME:-lo0}"

python -c "import torch" 2>/dev/null || {
  echo "need torch: uv pip install torch" >&2
  exit 1
}

exec torchrun --standalone --nnodes=1 --nproc_per_node="$NPROC" \
  --master_addr=127.0.0.1 --master_port="$MASTER_PORT" --local_addr=127.0.0.1 \
  examples/crash_torchrun_demo.py --mode "$MODE" --crash-rank "$CRASH_RANK"

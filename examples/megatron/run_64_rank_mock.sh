#!/usr/bin/env bash
# Laptop-friendly 64-rank Megatron fixture: 8 nodes x 8 ranks, TP2 PP4 DP8 SP2.
set -euo pipefail

cd "$(dirname "$0")/../.."
[[ -f .venv/bin/activate ]] && source .venv/bin/activate

PYTHON="${PYTHON:-python}"
MASTER_ADDR="${MASTER_ADDR:-127.0.0.1}"
MASTER_PORT="${MASTER_PORT:-29584}"

export PROBING_PORT="${PROBING_PORT:-18080}"
export PROBING=0
export PROBING_TORCHRUN_CLUSTER=1
export PROBING_ADVERTISE_ADDR="${PROBING_ADVERTISE_ADDR:-127.0.0.1}"
export PROBING_CLUSTER_REPORT_INTERVAL_SEC="${PROBING_CLUSTER_REPORT_INTERVAL_SEC:-1}"
export PROBING_CLUSTER_REPORT_MAX_INTERVAL_SEC="${PROBING_CLUSTER_REPORT_MAX_INTERVAL_SEC:-4}"
export PROBING_CLUSTER_REPORT_BACKOFF="${PROBING_CLUSTER_REPORT_BACKOFF:-0}"
export PROBING_CLUSTER_STALE_SEC="${PROBING_CLUSTER_STALE_SEC:-15}"
export MASTER_ADDR MASTER_PORT
export PROBING_MOCK_PYTHON="$PYTHON"

"$PYTHON" -c "import torch" 2>/dev/null || {
  echo "error: torch is required to launch the fixture with torchrun" >&2
  exit 1
}

echo "runtime: torchrun 64 workers · 8 logical nodes × 8 ranks · TCPStore ${MASTER_ADDR}:${MASTER_PORT}"
exec "$PYTHON" -m torch.distributed.run \
  --nnodes=1 \
  --node-rank=0 \
  --nproc-per-node=64 \
  --master-addr="$MASTER_ADDR" \
  --master-port="$MASTER_PORT" \
  --local-addr=127.0.0.1 \
  --no-python \
  examples/megatron/run_64_rank_worker.sh \
  --port "$PROBING_PORT" \
  --duration "${DURATION_SEC:-0}" \
  "$@"

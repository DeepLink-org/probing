#!/usr/bin/env bash
# ImageNet synthetic training — default 2-rank DDP via torchrun + probing cluster demo.
#
# Exercises distributed cluster query, distributed flamegraph, and federated
# profile_hotspot SQL (global.python.*) while training runs.
#
# Prerequisites:
#   make develop
#   uv pip install torch torchvision
#
# Usage:
#   ./examples/run_imagenet_ddp.sh
#   DURATION_SEC=60 ./examples/run_imagenet_ddp.sh
#   NPROC=2 DIST_BACKEND=nccl ./examples/run_imagenet_ddp.sh   # needs >=2 GPUs
#
# While training (rank 0):
#   http://127.0.0.1:${PROBING_PORT}/
#
# After a short run, try (another terminal):
#   probing -t 127.0.0.1:${PROBING_PORT} cluster nodes
#   probing -t 127.0.0.1:${PROBING_PORT} cluster query \
#     "SELECT rank, bucket_kind, bucket_name, self_us FROM global.python.profile_hotspot LIMIT 20"
#   probing -t 127.0.0.1:${PROBING_PORT} query \
#     "SELECT rank, name, self_us FROM python.torch_trace ORDER BY self_us DESC LIMIT 10"

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

[[ -f .venv/bin/activate ]] && source .venv/bin/activate
PYTHON="$(command -v "${PYTHON:-python}")"

if ! "$PYTHON" python/probing/dev_pth.py status >/dev/null 2>&1; then
  echo "error: dev env not ready — run: make develop" >&2
  exit 1
fi

# Always launch via the venv interpreter. Bare ``torchrun`` on PATH often points at
# pyenv/system Python, so worker ranks miss editable ``probing`` + site hooks.
TORCHRUN=("$PYTHON" -m torch.distributed.run)

NPROC="${NPROC:-2}"
DIST_BACKEND="${DIST_BACKEND:-gloo}"
DURATION_SEC="${DURATION_SEC:-120}"
MAX_STEPS="${MAX_STEPS:-0}"
BATCH_SIZE="${BATCH_SIZE:-64}"
WORKERS="${WORKERS:-0}"
PROBING_PORT="${PROBING_PORT:-18080}"
PROBING_DATA_DIR="${PROBING_DATA_DIR:-/tmp/probing_imagenet_ddp_$$}"
MASTER_PORT="${MASTER_PORT:-29582}"
SOAK_ASSERT="${SOAK_ASSERT:-1}"
MASTER_ADDR="${MASTER_ADDR:-127.0.0.1}"

export MASTER_ADDR MASTER_PORT
if [[ "$(uname -s)" == Darwin ]]; then
  export GLOO_SOCKET_IFNAME="${GLOO_SOCKET_IFNAME:-lo0}"
fi

export PROBING="${PROBING:-1}"
export PROBING_PORT
export PROBING_DATA_DIR
export PROBING_TORCH_PROFILING="${PROBING_TORCH_PROFILING:-0.01,backward=on}"
export PROBING_SAMPLE_RATE="${PROBING_SAMPLE_RATE:-1.0}"
export PROBING_RETENTION_DAYS="${PROBING_RETENTION_DAYS:-1}"
export SOAK_EXPECT_CLUSTER=1

mkdir -p "$PROBING_DATA_DIR"

# Rank 0 binds PROBING_PORT for the Web UI. A stale listener from a prior run
# causes "Address already in use" and the UI keeps talking to the old binary.
if command -v lsof >/dev/null 2>&1; then
  stale_pids="$(lsof -tiTCP:"$PROBING_PORT" -sTCP:LISTEN 2>/dev/null || true)"
  if [[ -n "$stale_pids" ]]; then
    echo "warning: port ${PROBING_PORT} in use (PIDs: ${stale_pids//$'\n'/, }) — stopping stale listener(s)"
  fi
  while read -r pid; do
    [[ -n "$pid" ]] || continue
    kill "$pid" 2>/dev/null || true
  done <<< "$stale_pids"
  sleep 0.5
  stale_pids="$(lsof -tiTCP:"$PROBING_PORT" -sTCP:LISTEN 2>/dev/null || true)"
  while read -r pid; do
    [[ -n "$pid" ]] || continue
    kill -9 "$pid" 2>/dev/null || true
  done <<< "$stale_pids"
fi

COMMON_ARGS=(
  examples/imagenet_with_span.py
  /tmp/imagenet-dummy
  --dummy
  --no-validate
  --arch resnet18
  --batch-size "$BATCH_SIZE"
  --workers "$WORKERS"
  --epochs 9999
  --max-duration-sec "$DURATION_SEC"
  --dist-backend "$DIST_BACKEND"
)

if [[ "$MAX_STEPS" -gt 0 ]]; then
  COMMON_ARGS+=(--max-steps "$MAX_STEPS")
fi

if [[ "$SOAK_ASSERT" == "1" ]]; then
  COMMON_ARGS+=(--soak-assert)
fi

if [[ "$DIST_BACKEND" == "nccl" ]]; then
  if ! "$PYTHON" -c "import torch; assert torch.cuda.is_available()" 2>/dev/null; then
    echo "error: nccl requires CUDA; use DIST_BACKEND=gloo on CPU/macOS" >&2
    exit 1
  fi
  gpu_count="$("$PYTHON" -c 'import torch; print(torch.cuda.device_count())')"
  if [[ "$NPROC" -gt "$gpu_count" ]]; then
    echo "error: nccl needs >= $NPROC GPUs (found $gpu_count)" >&2
    exit 1
  fi
fi

echo "=== ImageNet DDP + probing (${NPROC} ranks, backend=${DIST_BACKEND}) ==="
echo "PROBING=$PROBING  PROBING_PORT=$PROBING_PORT  PROBING_DATA_DIR=$PROBING_DATA_DIR"
echo "python=$PYTHON"
echo "limits: DURATION_SEC=$DURATION_SEC  MAX_STEPS=${MAX_STEPS:-0}"
echo "Web UI (rank 0): http://127.0.0.1:${PROBING_PORT}/"
echo

"${TORCHRUN[@]}" --standalone --nproc_per_node="$NPROC" \
  --master_addr="$MASTER_ADDR" \
  --master_port="$MASTER_PORT" \
  --local_addr=127.0.0.1 \
  "${COMMON_ARGS[@]}" "$@"

echo
echo "=== training finished ==="
echo "cluster nodes:"
echo "  probing -t 127.0.0.1:${PROBING_PORT} cluster nodes"
echo
echo "distributed torch_trace (flamegraph source):"
echo "  probing -t 127.0.0.1:${PROBING_PORT} cluster query \\"
echo "    \"SELECT rank, name, round(self_us/1e3, 2) AS self_ms FROM global.python.torch_trace ORDER BY self_us DESC LIMIT 20\""
echo
echo "federated profiler SQL (after a profile capture on each rank):"
echo "  probing -t 127.0.0.1:${PROBING_PORT} cluster query \\"
echo "    \"SELECT rank, bucket_kind, bucket_name, self_us FROM global.python.profile_hotspot ORDER BY self_us DESC LIMIT 20\""
echo
echo "Web: http://127.0.0.1:${PROBING_PORT}/stacks/distributed (full) · /stacks/distributed/py (Python only)"
echo "memtable data: $PROBING_DATA_DIR"

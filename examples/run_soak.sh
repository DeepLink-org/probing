#!/usr/bin/env bash
# Long-running probing soak: synthetic ImageNet training + post-run assertions.
#
# Usage (single process, CPU-friendly):
#   ./examples/run_soak.sh
#   DURATION_SEC=1200 ./examples/run_soak.sh
#
# Distributed (gloo on CPU, NCCL on CUDA):
#   NPROC=2 DIST_BACKEND=gloo ./examples/run_soak.sh
#
# Env:
#   DURATION_SEC     wall-clock cap (default 600)
#   MAX_STEPS        optimizer-step cap (default 0 = duration only)
#   NPROC            torchrun --nproc_per_node (default 1)
#   DIST_BACKEND     gloo (CPU) or nccl (CUDA, needs >=2 GPUs)
#   PROBING_DATA_DIR memtable root (default /tmp/probing_soak_$$)
#   SOAK_ASSERT      1 to run soak_assert.py after training (default 1)

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

DURATION_SEC="${DURATION_SEC:-600}"
MAX_STEPS="${MAX_STEPS:-0}"
NPROC="${NPROC:-1}"
DIST_BACKEND="${DIST_BACKEND:-gloo}"
PROBING_DATA_DIR="${PROBING_DATA_DIR:-/tmp/probing_soak_$$}"
SOAK_ASSERT="${SOAK_ASSERT:-1}"
BATCH_SIZE="${BATCH_SIZE:-64}"
WORKERS="${WORKERS:-0}"

export PROBING="${PROBING:-1}"
export PROBING_TORCH_PROFILING="${PROBING_TORCH_PROFILING:-on,shadow=4:1}"
export PROBING_SAMPLE_RATE="${PROBING_SAMPLE_RATE:-1.0}"
export PROBING_RETENTION_DAYS="${PROBING_RETENTION_DAYS:-1}"
export PROBING_DATA_DIR

if [[ "$NPROC" -gt 1 ]]; then
  export SOAK_EXPECT_CLUSTER=1
fi

mkdir -p "$PROBING_DATA_DIR"

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
)

if [[ "$MAX_STEPS" -gt 0 ]]; then
  COMMON_ARGS+=(--max-steps "$MAX_STEPS")
fi

if [[ "$SOAK_ASSERT" == "1" ]]; then
  COMMON_ARGS+=(--soak-assert)
fi

echo "=== probing soak ==="
echo "PROBING_DATA_DIR=$PROBING_DATA_DIR"
echo

if [[ "$NPROC" -le 1 ]]; then
  "${PYTHON:-python}" "${COMMON_ARGS[@]}"
else
  if [[ "$DIST_BACKEND" == "nccl" ]]; then
    if ! "${PYTHON:-python}" -c "import torch; assert torch.cuda.is_available()" 2>/dev/null; then
      echo "nccl soak requires CUDA; set DIST_BACKEND=gloo or NPROC=1" >&2
      exit 1
    fi
    if [[ "$NPROC" -gt 1 ]] && [[ "$(python -c 'import torch; print(torch.cuda.device_count())')" -lt "$NPROC" ]]; then
      echo "nccl soak needs >= $NPROC GPUs" >&2
      exit 1
    fi
  fi
  torchrun --standalone --nproc_per_node="$NPROC" \
    "${COMMON_ARGS[@]}" \
    --dist-backend "$DIST_BACKEND"
fi

echo
echo "soak complete (data: $PROBING_DATA_DIR)"

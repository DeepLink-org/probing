#!/usr/bin/env bash
# macOS / 无 CUDA：probing.fakes scripted Megatron 循环（非真 forward）。
#
#   ./examples/megatron/run_fakes.sh
#   TRAIN_ITERS=4 ./examples/megatron/run_fakes.sh
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"
[[ -f .venv/bin/activate ]] && source .venv/bin/activate
export PROBING="${PROBING:-1}"
export PROBING_FAKES="${PROBING_FAKES:-1}"
export PROBING_FAKES_FORCE="${PROBING_FAKES_FORCE:-1}"
export PROBING_FAKE_DEVICE="${PROBING_FAKE_DEVICE:-meta}"
ITERS="${TRAIN_ITERS:-4}"
exec python examples/megatron/pretrain_gpt.py --train-iters "$ITERS" "$@"

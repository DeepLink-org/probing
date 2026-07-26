#!/usr/bin/env bash
# 真实 Megatron-LM pretrain_gpt.py + 底层 fake（默认 ../Megatron-LM）。
#
#   ./examples/megatron/run_real_lm.sh
#   TRAIN_ITERS=2 MEGATRON_LM=/path/to/Megatron-LM ./examples/megatron/run_real_lm.sh
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"
[[ -f .venv/bin/activate ]] && source .venv/bin/activate
export PROBING="${PROBING:-1}"
ITERS="${TRAIN_ITERS:-2}"
exec python examples/megatron/run_megatron_lm_pretrain.py --train-iters "$ITERS" "$@"

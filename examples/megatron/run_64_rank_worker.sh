#!/usr/bin/env bash
# Configure the logical 8-rank node before Python startup hooks import Probing.
set -euo pipefail

: "${RANK:?torchrun must provide RANK}"
: "${PROBING_MOCK_PYTHON:?run through run_64_rank_mock.sh}"

logical_local_rank=$((RANK % 8))
logical_node_rank=$((RANK / 8))
tp_rank=$((RANK % 2))
pp_rank=$(((RANK / 2) % 4))
dp_rank=$((RANK / 8))

export LOCAL_RANK="$logical_local_rank"
export LOCAL_WORLD_SIZE=8
export GROUP_RANK="$logical_node_rank"
export NODE_RANK="$logical_node_rank"
export GROUP_WORLD_SIZE=8
export ROLE_NAME=trainer
export ROLE_RANK="$RANK"
export ROLE_WORLD_SIZE=64
printf -v PROBING_NODE_HOST 'megatron-node-%02d' "$logical_node_rank"
export PROBING_NODE_HOST
export PROBING_NODE_ROLE="dp=${dp_rank},pp=${pp_rank},sp=${tp_rank},tp=${tp_rank}"

exec "$PROBING_MOCK_PYTHON" examples/megatron/megatron_64_rank_mock.py "$@"

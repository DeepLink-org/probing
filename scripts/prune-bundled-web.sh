#!/usr/bin/env bash
# Drop stale dx hashed assets after `dx bundle` — keep only what index.html loads.
set -euo pipefail

PUBLIC="${1:-python/probing/bundled_web/public}"
INDEX="$PUBLIC/index.html"
ASSETS="$PUBLIC/assets"

if [[ ! -f "$INDEX" ]]; then
  exit 0
fi

entry_js="$(grep -oE 'web-dxh[^"'"'"' ]+\.js' "$INDEX" | head -1 || true)"
if [[ -z "$entry_js" ]]; then
  exit 0
fi

js_path="$ASSETS/$entry_js"
wasm_ref=""
if [[ -f "$js_path" ]]; then
  wasm_ref="$(grep -oE 'web_bg-dxh[0-9a-f]+\.wasm' "$js_path" | head -1 || true)"
fi

declare -a keep=(
  "$ASSETS/tailwind.css"
  "$ASSETS/logo.svg"
  "$PUBLIC/logo.svg"
  "$js_path"
  "$js_path.br"
)
if [[ -n "$wasm_ref" ]]; then
  keep+=("$ASSETS/$wasm_ref" "$ASSETS/${wasm_ref}.br")
fi

pruned=0
shopt -s nullglob
for f in "$ASSETS"/web-dxh* "$ASSETS"/web_bg-dxh*; do
  keep_it=0
  for k in "${keep[@]}"; do
    if [[ "$f" == "$k" ]]; then
      keep_it=1
      break
    fi
  done
  if [[ "$keep_it" -eq 0 ]]; then
    rm -f "$f"
    pruned=$((pruned + 1))
  fi
done
shopt -u nullglob

if [[ "$pruned" -gt 0 ]]; then
  echo "pruned $pruned stale web asset(s) under $ASSETS"
fi

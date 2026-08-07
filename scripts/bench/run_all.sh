#!/usr/bin/env bash
# Run all (or selected) azalea probe scripts against a live MC server.
# Usage: ./run_all.sh <port> [script1.json script2.json ...]
set -euo pipefail

PORT="${1:-4444}"
shift || true

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
PROBE_DIR="$ROOT/scripts/probe"
TS="$(date +%Y%m%d_%H%M%S)"
OUT="$ROOT/bench_results_${TS}.txt"

if [ $# -gt 0 ]; then
  SCRIPTS=("$@")
else
  SCRIPTS=("$(ls "$PROBE_DIR"/*.json | xargs -n1 basename)")
fi

echo "=== SeekerCraft bench run @ $(date) ===" | tee "$OUT"
echo "port=$PORT scripts=${#SCRIPTS[@]}" | tee -a "$OUT"

PASS=0
FAIL=0
for s in "${SCRIPTS[@]}"; do
  p="$PROBE_DIR/$s"
  if [ ! -f "$p" ]; then
    echo "SKIP  $s (not found)" | tee -a "$OUT"
    continue
  fi
  if cargo run -q -p craft-agent-minecraft --example azalea_probe \
      --features azalea-bot -- "$PORT" --script "$p" > /dev/null 2>&1; then
    echo "PASS  $s" | tee -a "$OUT"
    PASS=$((PASS+1))
  else
    echo "FAIL  $s" | tee -a "$OUT"
    FAIL=$((FAIL+1))
  fi
done

echo "=== done: pass=$PASS fail=$FAIL (see bench_results_${TS}.txt) ===" | tee -a "$OUT"
[ "$FAIL" -eq 0 ]

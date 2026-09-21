#!/usr/bin/env bash
# Headless AI-agent profiling harness (deterministic, mock Pixiv API).
#
# Runs the real app update/view/async pipeline against a local mock server,
# simulates a scripted user session (tabs, scrolling, viewer, artist,
# filters, search, downloads), and writes a profiling report.
#
# Usage:
#   ./scripts/profile.sh [--out DIR] [--cpu HZ] [--dev] [--no-cpu]
#
#   --out DIR   report output directory        (default: profiles/latest)
#   --cpu HZ    CPU sampling frequency         (default: 250, 0 = off)
#   --dev       use the fast dev build (no line-level CPU symbols)
#   --no-cpu    disable CPU sampling
#
# Outputs in DIR: report.md, report.json, flamegraph.svg, hotspots.md
set -euo pipefail
cd "$(dirname "$0")/.."

OUT="profiles/latest"
CPU="250"
PROFILE_FLAGS="--profile profiling"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --out) OUT="$2"; shift 2 ;;
    --cpu) CPU="$2"; shift 2 ;;
    --no-cpu) CPU="0"; shift ;;
    --dev) PROFILE_FLAGS=""; shift ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

mkdir -p "$OUT"
echo "=== pixiv-manager headless profiler ==="
echo "output dir : $OUT"
echo "cpu freq   : ${CPU} Hz"
echo "profile    : ${PROFILE_FLAGS:-dev}"

exec cargo run -p pixiv-manager $PROFILE_FLAGS --features profiling \
  --example profiler -- --out "$OUT" --cpu "$CPU"
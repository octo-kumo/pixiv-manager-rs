#!/usr/bin/env bash
# Real-GUI end-to-end profiling: runs the actual windowed app, drives real
# mouse/keyboard input with xdotool, and captures CPU + app telemetry.
#
# Uses the app's built-in profiling HTTP control server (pprof sampling) so
# no external tooling beyond `xdotool` is required. The app uses your REAL
# Pixiv config/token (set --temp to profile offline instead).
#
# Usage:
#   ./scripts/profile-gui.sh [--out DIR] [--port PORT] [--secs N] [--temp]
#
#   --out DIR   report output directory        (default: profiles/live)
#   --port PORT HTTP control server port       (default: 18765)
#   --secs N    how long to drive input        (default: 20)
#   --temp      use a throwaway config dir (offline onboarding) instead of
#               your real token
#
# Outputs in DIR: report.md, report.json, flamegraph.svg, hotspots.md
set -euo pipefail
cd "$(dirname "$0")/.."

OUT="profiles/live"
PORT="18765"
SECS="20"
TEMP=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --out) OUT="$2"; shift 2 ;;
    --port) PORT="$2"; shift 2 ;;
    --secs) SECS="$2"; shift 2 ;;
    --temp) TEMP=1; shift ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

if ! command -v xdotool >/dev/null 2>&1; then
  echo "xdotool is required (install it: sudo dnf install xdotool)" >&2
  exit 2
fi

mkdir -p "$OUT"
echo "=== pixiv-manager GUI profiler ==="
echo "output dir : $OUT"
echo "control    : http://127.0.0.1:$PORT"

# Build the profiling binary (optimized + debug symbols).
cargo build -p pixiv-manager --profile profiling --features profiling

# Config: real token by default, or a throwaway temp dir for --temp.
if [[ $TEMP -eq 1 ]]; then
  export PIXIV_MANAGER_CONFIG_DIR="$(mktemp -d)"
  echo "config dir : (temporary, offline)"
else
  echo "config dir : $HOME/.config/pixiv-manager (real token — live Pixiv)"
fi

export PIXIV_MANAGER_PROFILE_PORT="$PORT"
export PIXIV_MANAGER_PROFILE_DIR="$(pwd)/$OUT"
export RUST_LOG=info

# Start the app.
./target/profiling/pixiv-manager >"$OUT/app.log" 2>&1 &
APP_PID=$!
echo "app pid    : $APP_PID (log: $OUT/app.log)"
trap 'kill $APP_PID 2>/dev/null || true' EXIT

# Wait for the control server.
for i in $(seq 1 50); do
  if curl -s "http://127.0.0.1:$PORT/metrics" >/dev/null 2>&1; then break; fi
  sleep 0.2
done
if ! curl -s "http://127.0.0.1:$PORT/metrics" >/dev/null 2>&1; then
  echo "app did not start its control server in time; see $OUT/app.log" >&2
  exit 1
fi

# Start CPU sampling.
curl -s "http://127.0.0.1:$PORT/profile/start?freq=250" >/dev/null
echo "CPU sampling started (250Hz)"

# Find the window and drive real input: mouse sweeps over the gallery
# (parallax + hover redraws), scroll wheel, and a few keys.
WINDOW_ID=""
for i in $(seq 1 25); do
  WINDOW_ID="$(xdotool search --name 'Pixiv' 2>/dev/null | head -1 || true)"
  [[ -n "$WINDOW_ID" ]] && break
  sleep 0.2
done

if [[ -n "$WINDOW_ID" ]]; then
  echo "driving window $WINDOW_ID for ${SECS}s ..."
  xdotool windowactivate --sync "$WINDOW_ID" 2>/dev/null || true
  xdotool windowsize "$WINDOW_ID" 1600 900 2>/dev/null || true

  END=$((SECONDS + SECS))
  SWEEP=0
  while [[ $SECONDS -lt $END ]]; do
    # Sweep the mouse across the gallery (horizontal + vertical).
    X=$(( (SWEEP * 37) % 1500 + 50 ))
    Y=$(( (SWEEP * 53) % 700 + 120 ))
    xdotool mousemove --sync "$X" "$Y" 2>/dev/null || true
    if (( SWEEP % 8 == 0 )); then
      xdotool click 4 2>/dev/null || true   # scroll up
      xdotool click 5 2>/dev/null || true   # scroll down
    fi
    if (( SWEEP % 25 == 0 )); then
      xdotool key F3 2>/dev/null || true    # debug overlay
    fi
    SWEEP=$((SWEEP + 1))
    sleep 0.15
  done
else
  echo "window not found; running idle for ${SECS}s"
  sleep "$SECS"
fi

# Stop sampling and write the report.
echo "stopping capture ..."
curl -s "http://127.0.0.1:$PORT/profile/stop" | head -30
curl -s "http://127.0.0.1:$PORT/report" >/dev/null
echo
echo "=== artifacts in $OUT ==="
ls -la "$OUT"
echo
echo "summary: $OUT/report.md  ·  flamegraph: $OUT/flamegraph.svg"
kill "$APP_PID" 2>/dev/null || true
trap - EXIT
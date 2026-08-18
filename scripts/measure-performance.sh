#!/usr/bin/env bash
# measure-performance.sh — Measure Maintainer performance against defined budgets.
#
# Usage:
#   scripts/measure-performance.sh [path-to-binary]
#
# If no binary path is given, defaults to target/release/maintainer-app.
#
# Measures:
#   - Cold start time (wall clock from exec to first GTK window mapped)
#   - Steady-state RSS after 30 seconds idle
#   - 5-minute average CPU usage (idle, Overview page visible)
#
# Exit codes:
#   0 — All hard ceilings met
#   1 — One or more hard ceilings breached
#   2 — Script error (missing dependencies, binary not found, etc.)
#
# Environment:
#   PERF_DURATION_SECS   — CPU measurement window (default: 300, i.e. 5 minutes)
#   PERF_SETTLE_SECS     — Time to wait before RSS measurement (default: 30)
#   PERF_DISPLAY         — X11 display for xvfb (default: :99)
#   PERF_SKIP_XVFB       — Set to 1 to use the current display instead of xvfb
#
# Requirements: xvfb-run or Xvfb, ps, bc, time (GNU coreutils)

set -euo pipefail

# --- Configuration ---
BINARY="${1:-target/release/maintainer-app}"
DURATION="${PERF_DURATION_SECS:-300}"
SETTLE="${PERF_SETTLE_SECS:-30}"
DISPLAY_NUM="${PERF_DISPLAY:-:99}"
SKIP_XVFB="${PERF_SKIP_XVFB:-0}"

# Hard ceilings (from PRD)
RSS_CEILING_KB=$((100 * 1024))   # 100 MB in KB
CPU_CEILING="0.5"                 # percent
STARTUP_CEILING_MS=2000           # milliseconds

# Targets (reported but not enforced as exit code)
RSS_TARGET_KB=$((70 * 1024))     # 70 MB in KB

# --- Helpers ---
die() { echo "ERROR: $*" >&2; exit 2; }
info() { echo "==> $*"; }
warn() { echo "WARNING: $*" >&2; }

check_deps() {
    for cmd in ps bc; do
        command -v "$cmd" >/dev/null 2>&1 || die "Required command not found: $cmd"
    done
    if [[ "$SKIP_XVFB" != "1" ]]; then
        command -v Xvfb >/dev/null 2>&1 || command -v xvfb-run >/dev/null 2>&1 || \
            die "Xvfb or xvfb-run required (install xvfb package)"
    fi
}

cleanup() {
    local exit_code=$?
    if [[ -n "${APP_PID:-}" ]] && kill -0 "$APP_PID" 2>/dev/null; then
        kill "$APP_PID" 2>/dev/null || true
        wait "$APP_PID" 2>/dev/null || true
    fi
    if [[ -n "${XVFB_PID:-}" ]] && kill -0 "$XVFB_PID" 2>/dev/null; then
        kill "$XVFB_PID" 2>/dev/null || true
    fi
    exit $exit_code
}
trap cleanup EXIT

# --- Validation ---
check_deps
[[ -x "$BINARY" ]] || die "Binary not found or not executable: $BINARY"

info "Maintainer Performance Measurement"
info "Binary: $BINARY"
info "CPU sample duration: ${DURATION}s"
info "RSS settle time: ${SETTLE}s"
info "Hard ceilings: RSS ≤ 100 MB, CPU ≤ 0.5%, startup ≤ 2000 ms"
info "Target: RSS ≤ 70 MB"
echo ""

# --- Start display server if needed ---
if [[ "$SKIP_XVFB" != "1" ]]; then
    info "Starting Xvfb on $DISPLAY_NUM"
    Xvfb "$DISPLAY_NUM" -screen 0 1920x1080x24 &
    XVFB_PID=$!
    sleep 1
    if ! kill -0 "$XVFB_PID" 2>/dev/null; then
        die "Xvfb failed to start"
    fi
    export DISPLAY="$DISPLAY_NUM"
else
    info "Using existing display: ${DISPLAY:-:0}"
fi

# --- Measure cold start time ---
info "Measuring cold start time..."

# Drop filesystem caches if running as root (optional, best-effort)
if [[ $EUID -eq 0 ]]; then
    sync
    echo 3 > /proc/sys/vm/drop_caches 2>/dev/null || true
fi

START_NS=$(date +%s%N)
"$BINARY" &
APP_PID=$!

# Wait for the process to be alive and its RSS to appear (proxy for window mapped)
STARTUP_TIMEOUT=10
ELAPSED_MS=0
while [[ $ELAPSED_MS -lt $((STARTUP_TIMEOUT * 1000)) ]]; do
    if ps -p "$APP_PID" -o rss= 2>/dev/null | grep -q '[0-9]'; then
        RSS_VAL=$(ps -p "$APP_PID" -o rss= 2>/dev/null | tr -d ' ')
        if [[ "${RSS_VAL:-0}" -gt 1024 ]]; then
            break
        fi
    fi
    sleep 0.05
    NOW_NS=$(date +%s%N)
    ELAPSED_MS=$(( (NOW_NS - START_NS) / 1000000 ))
done

if ! kill -0 "$APP_PID" 2>/dev/null; then
    die "Application crashed during startup"
fi

STARTUP_MS=$ELAPSED_MS
info "Cold start: ${STARTUP_MS} ms"

# --- Measure steady-state RSS ---
info "Waiting ${SETTLE}s for steady state..."
sleep "$SETTLE"

if ! kill -0 "$APP_PID" 2>/dev/null; then
    die "Application exited during settle period"
fi

RSS_KB=$(ps -p "$APP_PID" -o rss= 2>/dev/null | tr -d ' ')
RSS_MB=$(echo "scale=1; ${RSS_KB:-0} / 1024" | bc)
info "Steady RSS: ${RSS_MB} MB (${RSS_KB} KB)"

# --- Measure 5-minute average CPU ---
info "Measuring CPU over ${DURATION}s..."

# Sample CPU at start
CPU_START=$(ps -p "$APP_PID" -o cputime= 2>/dev/null | tr -d ' ')
WALL_START=$(date +%s)

sleep "$DURATION"

if ! kill -0 "$APP_PID" 2>/dev/null; then
    die "Application exited during CPU measurement"
fi

CPU_END=$(ps -p "$APP_PID" -o cputime= 2>/dev/null | tr -d ' ')
WALL_END=$(date +%s)

# Convert cputime (HH:MM:SS or MM:SS) to seconds
cputime_to_secs() {
    local t="$1"
    local h=0 m=0 s=0
    if [[ "$t" =~ ^([0-9]+):([0-9]+):([0-9]+)$ ]]; then
        h="${BASH_REMATCH[1]}"
        m="${BASH_REMATCH[2]}"
        s="${BASH_REMATCH[3]}"
    elif [[ "$t" =~ ^([0-9]+):([0-9]+)$ ]]; then
        m="${BASH_REMATCH[1]}"
        s="${BASH_REMATCH[2]}"
    else
        echo "0"
        return
    fi
    echo $(( h * 3600 + m * 60 + s ))
}

CPU_START_S=$(cputime_to_secs "$CPU_START")
CPU_END_S=$(cputime_to_secs "$CPU_END")
WALL_ELAPSED=$(( WALL_END - WALL_START ))

if [[ $WALL_ELAPSED -gt 0 ]]; then
    CPU_PCT=$(echo "scale=2; ($CPU_END_S - $CPU_START_S) * 100 / $WALL_ELAPSED" | bc)
else
    CPU_PCT="0.00"
fi

info "Average CPU: ${CPU_PCT}% over ${WALL_ELAPSED}s"

# --- Stop the application ---
kill "$APP_PID" 2>/dev/null || true
wait "$APP_PID" 2>/dev/null || true
unset APP_PID

# --- Report ---
echo ""
echo "╔══════════════════════════════════════════════════════════╗"
echo "║           MAINTAINER PERFORMANCE REPORT                 ║"
echo "╠══════════════════════════════════════════════════════════╣"
printf "║  Cold start:    %6d ms   (ceiling: %d ms)       ║\n" "$STARTUP_MS" "$STARTUP_CEILING_MS"
printf "║  Steady RSS:    %6.1f MB   (ceiling: 100 MB)          ║\n" "$RSS_MB"
printf "║  Avg CPU:       %6.2f %%    (ceiling: 0.50%%)          ║\n" "$CPU_PCT"
echo "╠══════════════════════════════════════════════════════════╣"

# --- Target check (informational) ---
BREACH=0
TARGET_NOTE=""

if [[ "${RSS_KB:-0}" -gt "$RSS_TARGET_KB" ]]; then
    TARGET_NOTE="  NOTE: RSS exceeds 70 MB target (not a hard failure)"
    echo "║  TARGET: RSS ${RSS_MB} MB exceeds 70 MB target         ║"
fi

# --- Hard ceiling checks ---
FAIL=0

if [[ "$STARTUP_MS" -gt "$STARTUP_CEILING_MS" ]]; then
    echo "║  FAIL: Startup ${STARTUP_MS} ms exceeds 2000 ms ceiling  ║"
    FAIL=1
fi

if [[ "${RSS_KB:-0}" -gt "$RSS_CEILING_KB" ]]; then
    echo "║  FAIL: RSS ${RSS_MB} MB exceeds 100 MB ceiling         ║"
    FAIL=1
fi

CPU_OVER=$(echo "$CPU_PCT > $CPU_CEILING" | bc -l 2>/dev/null || echo "0")
if [[ "$CPU_OVER" == "1" ]]; then
    echo "║  FAIL: CPU ${CPU_PCT}% exceeds 0.5% ceiling            ║"
    FAIL=1
fi

if [[ $FAIL -eq 0 ]]; then
    echo "║  RESULT: ALL HARD CEILINGS MET ✓                       ║"
else
    echo "║  RESULT: HARD CEILING BREACHED ✗                       ║"
fi

echo "╚══════════════════════════════════════════════════════════╝"

if [[ -n "$TARGET_NOTE" ]]; then
    echo ""
    echo "$TARGET_NOTE"
fi

exit $FAIL

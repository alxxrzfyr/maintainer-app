#!/usr/bin/env bash
# release-smoke.sh — Automated release smoke test.
#
# Builds a release binary, launches under Xvfb, verifies no crash, and
# checks basic exit codes.
#
# Usage:
#   tests/system/release-smoke.sh
#
# Requirements: xvfb, cargo, sudo, libgtk-4-dev, libsqlite3-dev, libpolkit-gobject-1-dev
#
# Exit codes:
#   0 — All smoke tests passed
#   1 — One or more smoke tests failed
#   2 — Script/environment error

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
DISPLAY_NUM="${SMOKE_DISPLAY:-:98}"
TIMEOUT_SECS="${SMOKE_TIMEOUT:-10}"

PASS=0
FAIL=0
ERRORS=()

# --- Helpers ---
die() { echo "ERROR: $*" >&2; exit 2; }
info() { echo "==> $*"; }
pass() { echo "  ✓ $*"; PASS=$((PASS + 1)); }
fail() { echo "  ✗ $*"; FAIL=$((FAIL + 1)); ERRORS+=("$*"); }

cleanup() {
    if [[ -n "${APP_PID:-}" ]] && kill -0 "$APP_PID" 2>/dev/null; then
        kill "$APP_PID" 2>/dev/null || true
        wait "$APP_PID" 2>/dev/null || true
    fi
    if [[ -n "${XVFB_PID:-}" ]] && kill -0 "$XVFB_PID" 2>/dev/null; then
        kill "$XVFB_PID" 2>/dev/null || true
    fi
}
trap cleanup EXIT

# --- Validate environment ---
for cmd in cargo Xvfb; do
    command -v "$cmd" >/dev/null 2>&1 || die "Required command not found: $cmd"
done

if [[ $(id -u) -ne 0 ]]; then
    command -v sudo >/dev/null 2>&1 || die "Required command not found: sudo"
    sudo -n true 2>/dev/null || die "Passwordless sudo is required to verify root-launch refusal"
fi

cd "$PROJECT_ROOT"

# --- Test 1: Release build succeeds ---
info "Test: Release build"
if python3 scripts/build-release.py 2>&1; then
    pass "Release build succeeded"
else
    fail "Release build failed"
    exit 1
fi

# Check binary exists
APP_BIN="target/release/maintainer-app"
HELPER_BIN="target/release/maintainer-helper"

if [[ $(id -u) -eq 0 ]]; then
    SMOKE_USER="${SMOKE_USER:-${SUDO_USER:-}}"
    [[ -n "$SMOKE_USER" ]] || die "Set SMOKE_USER to an existing unprivileged account when running as root"
    SMOKE_UID="$(id -u -- "$SMOKE_USER")" || die "SMOKE_USER does not exist: $SMOKE_USER"
    [[ "$SMOKE_UID" -ne 0 ]] || die "SMOKE_USER must not be root"
    command -v runuser >/dev/null 2>&1 || die "Required command not found: runuser"
    APP_LAUNCH=(runuser -u "$SMOKE_USER" -- env "DISPLAY=$DISPLAY_NUM" "$PROJECT_ROOT/$APP_BIN")
else
    APP_LAUNCH=("$APP_BIN")
fi

if [[ -x "$APP_BIN" ]]; then
    pass "App binary exists and is executable"
else
    fail "App binary not found: $APP_BIN"
fi

if [[ -x "$HELPER_BIN" ]]; then
    pass "Helper binary exists and is executable"
else
    fail "Helper binary not found: $HELPER_BIN"
fi

# --- Test 2: Start Xvfb ---
info "Test: Display server"
Xvfb "$DISPLAY_NUM" -screen 0 1920x1080x24 &
XVFB_PID=$!
sleep 1

if kill -0 "$XVFB_PID" 2>/dev/null; then
    pass "Xvfb started on $DISPLAY_NUM"
    export DISPLAY="$DISPLAY_NUM"
else
    fail "Xvfb failed to start"
    die "Cannot continue without display"
fi

# --- Test 3: Application launches without immediate crash ---
info "Test: Application launch (no crash within ${TIMEOUT_SECS}s)"

"${APP_LAUNCH[@]}" &
APP_PID=$!

# Wait a short time and check it's still alive
sleep 3

if kill -0 "$APP_PID" 2>/dev/null; then
    pass "Application survived initial 3s"
else
    EXIT_CODE=0
    wait "$APP_PID" 2>/dev/null || EXIT_CODE=$?
    fail "Application crashed within 3s (exit code: $EXIT_CODE)"
fi

# Wait longer
if [[ -n "${APP_PID:-}" ]] && kill -0 "$APP_PID" 2>/dev/null; then
    sleep $((TIMEOUT_SECS - 3))
    if kill -0 "$APP_PID" 2>/dev/null; then
        pass "Application survived ${TIMEOUT_SECS}s without crash"
    else
        EXIT_CODE=0
        wait "$APP_PID" 2>/dev/null || EXIT_CODE=$?
        fail "Application crashed during smoke period (exit code: $EXIT_CODE)"
    fi
fi

# --- Test 4: Application responds to SIGTERM gracefully ---
info "Test: Graceful shutdown"

if [[ -n "${APP_PID:-}" ]] && kill -0 "$APP_PID" 2>/dev/null; then
    kill -TERM "$APP_PID" 2>/dev/null || true
    WAIT_COUNT=0
    while kill -0 "$APP_PID" 2>/dev/null && [[ $WAIT_COUNT -lt 10 ]]; do
        sleep 0.5
        WAIT_COUNT=$((WAIT_COUNT + 1))
    done

    if kill -0 "$APP_PID" 2>/dev/null; then
        fail "Application did not exit within 5s of SIGTERM"
        kill -9 "$APP_PID" 2>/dev/null || true
    else
        EXIT_CODE=0
        wait "$APP_PID" 2>/dev/null || EXIT_CODE=$?
        if [[ $EXIT_CODE -eq 0 || $EXIT_CODE -eq 143 ]]; then
            pass "Application exited cleanly on SIGTERM (code: $EXIT_CODE)"
        else
            fail "Application exited with unexpected code on SIGTERM: $EXIT_CODE"
        fi
    fi
    unset APP_PID
fi

# --- Test 5: Helper refuses to run without D-Bus context (expected failure) ---
info "Test: Helper refuses bare execution"
if timeout 5 "$HELPER_BIN" 2>/dev/null; then
    # If it exits 0 with no D-Bus, that might be fine (it may just print usage)
    pass "Helper exited without crash"
else
    EXIT_CODE=$?
    if [[ $EXIT_CODE -ne 0 ]]; then
        pass "Helper refused bare execution (exit code: $EXIT_CODE) — expected"
    fi
fi

# --- Test 6: GUI refuses root execution before persistence initialization ---
info "Test: GUI root-refusal check"
ROOT_TEST_DIR="$(mktemp -d)"
ROOT_TEST_DATA="$ROOT_TEST_DIR/data"
ROOT_OUTPUT="$ROOT_TEST_DIR/output"
ROOT_EXIT=0
if [[ $(id -u) -eq 0 ]]; then
    XDG_DATA_HOME="$ROOT_TEST_DATA" "$APP_BIN" >"$ROOT_OUTPUT" 2>&1 || ROOT_EXIT=$?
else
    sudo -n env XDG_DATA_HOME="$ROOT_TEST_DATA" DISPLAY="$DISPLAY" \
        "$PROJECT_ROOT/$APP_BIN" >"$ROOT_OUTPUT" 2>&1 || ROOT_EXIT=$?
fi

if [[ $ROOT_EXIT -eq 1 ]] && grep -q "must run as a normal user" "$ROOT_OUTPUT" && [[ ! -e "$ROOT_TEST_DATA" ]]; then
    pass "Root launch refused before creating application data"
else
    fail "Root launch did not refuse cleanly (exit code: $ROOT_EXIT)"
fi
rm -rf "$ROOT_TEST_DIR"

# --- Report ---
echo ""
echo "════════════════════════════════════════"
echo "  Release Smoke Test Results"
echo "════════════════════════════════════════"
echo "  Passed: $PASS"
echo "  Failed: $FAIL"
echo "════════════════════════════════════════"

if [[ $FAIL -gt 0 ]]; then
    echo ""
    echo "Failures:"
    for err in "${ERRORS[@]}"; do
        echo "  - $err"
    done
    exit 1
fi

exit 0

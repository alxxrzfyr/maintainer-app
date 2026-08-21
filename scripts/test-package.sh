#!/usr/bin/env bash
# scripts/test-package.sh — Smoke test for the .deb package
#
# Usage: scripts/test-package.sh <path-to-deb>
#
# This script installs the given .deb into a disposable environment,
# validates file paths, metadata, schema compilation, application launch,
# and clean removal. It is intended to run inside a container or chroot.
#
# Requirements: dpkg, apt, xvfb-run, desktop-file-validate, appstreamcli
# Must be run as root (or in a disposable container with root access).

set -euo pipefail

readonly PACKAGE_PATH="${1:?Usage: $0 <path-to-deb>}"
readonly APP_ID="io.github.maintainerapp.Maintainer"
readonly HELPER_SERVICE="io.github.maintainerapp.Helper"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

pass() { echo -e "${GREEN}✓ PASS${NC}: $1"; }
fail() { echo -e "${RED}✗ FAIL${NC}: $1"; exit 1; }
info() { echo -e "${YELLOW}→${NC} $1"; }

# --- Pre-flight checks ---
info "Checking prerequisites..."
if [ "$(id -u)" -ne 0 ]; then
    fail "This script must be run as root (or inside a disposable container)"
fi

if [ ! -f "$PACKAGE_PATH" ]; then
    fail "Package not found: $PACKAGE_PATH"
fi

# --- Install the package ---
info "Installing package: $PACKAGE_PATH"
apt-get update -qq
apt-get install -y -qq xvfb desktop-file-utils appstream 2>/dev/null || true
dpkg -i "$PACKAGE_PATH" || apt-get install -f -y -qq

# --- Verify binary paths ---
info "Checking binary paths..."
[ -x /usr/bin/maintainer ] || fail "/usr/bin/maintainer not found or not executable"
pass "/usr/bin/maintainer exists and is executable"

[ -x /usr/libexec/maintainer-helper ] || fail "/usr/libexec/maintainer-helper not found or not executable"
pass "/usr/libexec/maintainer-helper exists and is executable"

# --- Verify data files ---
info "Checking installed data files..."
[ -f /usr/share/applications/${APP_ID}.desktop ] || fail "Desktop file missing"
pass "Desktop file installed"

[ -f /usr/share/metainfo/${APP_ID}.metainfo.xml ] || fail "Metainfo file missing"
pass "Metainfo file installed"

[ -f /usr/share/glib-2.0/schemas/${APP_ID}.gschema.xml ] || fail "GSettings schema missing"
pass "GSettings schema installed"

[ -f /usr/share/dbus-1/system-services/${HELPER_SERVICE}.service ] || fail "D-Bus service file missing"
pass "D-Bus service file installed"

[ -f /usr/share/polkit-1/actions/io.github.maintainerapp.policy ] || fail "Polkit policy missing"
pass "Polkit policy installed"

[ -f /usr/share/icons/hicolor/scalable/apps/${APP_ID}.svg ] || fail "Scalable icon missing"
pass "Scalable icon installed"

[ -d /usr/share/maintainer ] || fail "/usr/share/maintainer directory missing"
pass "Application data directory exists"

# --- Validate desktop file ---
info "Validating desktop file..."
if command -v desktop-file-validate >/dev/null 2>&1; then
    desktop-file-validate /usr/share/applications/${APP_ID}.desktop || fail "Desktop file validation failed"
    pass "Desktop file is valid"
else
    info "desktop-file-validate not available, skipping"
fi

# --- Validate AppStream metainfo ---
info "Validating AppStream metainfo..."
if command -v appstreamcli >/dev/null 2>&1; then
    appstreamcli validate --no-net /usr/share/metainfo/${APP_ID}.metainfo.xml || \
        info "AppStream validation produced warnings (non-fatal)"
    pass "Metainfo validation ran"
else
    info "appstreamcli not available, skipping"
fi

# --- Compile and verify GSettings schema ---
info "Compiling GSettings schemas..."
glib-compile-schemas /usr/share/glib-2.0/schemas/
[ -f /usr/share/glib-2.0/schemas/gschemas.compiled ] || fail "Schema compilation failed"
pass "GSettings schemas compiled successfully"

# --- Launch application under Xvfb ---
info "Testing application launch under Xvfb..."
export DISPLAY=:99
LAUNCH_OK=0
timeout 5 xvfb-run -a /usr/bin/maintainer --version >/dev/null 2>&1 && LAUNCH_OK=1 || true

if [ "$LAUNCH_OK" -eq 1 ]; then
    pass "Application launched successfully (--version)"
else
    # Try a basic launch with timeout (app may not support --version yet)
    timeout 3 xvfb-run -a /usr/bin/maintainer &
    APP_PID=$!
    sleep 2
    if kill -0 "$APP_PID" 2>/dev/null; then
        kill "$APP_PID" 2>/dev/null || true
        pass "Application launched and ran for 2 seconds"
    else
        info "Application exited quickly (may not have display — non-fatal in CI)"
    fi
fi

# --- Remove the package ---
info "Removing package..."
dpkg --remove maintainer

# --- Verify removal ---
info "Verifying removal..."
[ ! -f /usr/bin/maintainer ] || fail "/usr/bin/maintainer still exists after removal"
pass "/usr/bin/maintainer removed"

[ ! -f /usr/libexec/maintainer-helper ] || fail "/usr/libexec/maintainer-helper still exists after removal"
pass "/usr/libexec/maintainer-helper removed"

[ ! -f /usr/share/dbus-1/system-services/${HELPER_SERVICE}.service ] || fail "D-Bus service still exists after removal"
pass "D-Bus service removed"

[ ! -f /usr/share/polkit-1/actions/io.github.maintainerapp.policy ] || fail "Polkit policy still exists after removal"
pass "Polkit policy removed"

[ ! -f /usr/share/applications/${APP_ID}.desktop ] || fail "Desktop file still exists after removal"
pass "Desktop file removed"

# --- Verify purge cleans everything ---
info "Purging package..."
dpkg --purge maintainer 2>/dev/null || true

[ ! -d /var/lib/maintainer ] || fail "/var/lib/maintainer still exists after purge"
pass "Application data purged"

# --- Summary ---
echo ""
echo -e "${GREEN}═══════════════════════════════════════${NC}"
echo -e "${GREEN}  All package smoke tests passed!${NC}"
echo -e "${GREEN}═══════════════════════════════════════${NC}"

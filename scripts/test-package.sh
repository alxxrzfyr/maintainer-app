#!/usr/bin/env bash
# Usage: MAINTAINER_PACKAGE_TEST_DISPOSABLE=1 scripts/test-package.sh target.deb --previous-deb previous.deb
# Destructive installed-package checks, exclusively for disposable systems.
set -euo pipefail

fail() { printf 'FAIL: %s\n' "$1" >&2; exit 1; }
pass() { printf 'PASS: %s\n' "$1"; }

[[ "${MAINTAINER_PACKAGE_TEST_DISPOSABLE:-}" == 1 ]] || fail 'Set MAINTAINER_PACKAGE_TEST_DISPOSABLE=1 only in a disposable system'
[[ "$(id -u)" == 0 ]] || fail 'Disposable installed-package checks require root'
[[ $# -ge 1 ]] || fail 'Usage: test-package.sh target.deb --previous-deb previous.deb'
TARGET_INPUT=$1
shift
PREVIOUS_INPUT=${MAINTAINER_PREVIOUS_DEB:-}
if [[ $# -gt 0 ]]; then
    [[ $# == 2 && $1 == --previous-deb ]] || fail 'Expected --previous-deb previous.deb'
    PREVIOUS_INPUT=$2
fi
[[ -n "$PREVIOUS_INPUT" ]] || fail 'Upgrade verification requires --previous-deb or MAINTAINER_PREVIOUS_DEB'

for tool in install systemctl timeout apt-get dpkg dpkg-deb dpkg-query sha256sum cmp cut readlink stat useradd userdel runuser getent mktemp python3 dbus-run-session xvfb-run Xvfb xauth busctl desktop-file-validate appstreamcli glib-compile-schemas gsettings grep rm chmod chown; do
    command -v "$tool" >/dev/null 2>&1 || fail "Missing prerequisite: $tool"
done
[[ -S /run/dbus/system_bus_socket ]] || fail 'A real system D-Bus is required'
busctl --system --no-pager status org.freedesktop.DBus >/dev/null 2>&1 || fail 'System D-Bus is unreachable'
busctl --system --no-pager status org.freedesktop.PolicyKit1 >/dev/null 2>&1 || fail 'A running system Polkit authority is required'
readonly TARGET=$(readlink -f -- "$TARGET_INPUT")
readonly PREVIOUS=$(readlink -f -- "$PREVIOUS_INPUT")
[[ -f "$TARGET" && -f "$PREVIOUS" ]] || fail 'Both package artifacts must exist'
[[ "$TARGET" != "$PREVIOUS" ]] || fail 'Previous and target artifacts must differ'
for artifact in "$PREVIOUS" "$TARGET"; do
    [[ "$(dpkg-deb -f "$artifact" Package)" == maintainer ]] || fail 'Unexpected package name'
    architecture=$(dpkg-deb -f "$artifact" Architecture)
    [[ "$architecture" == "$(dpkg --print-architecture)" || "$architecture" == all ]] || fail 'Package architecture does not match test system'
done
readonly TARGET_VERSION=$(dpkg-deb -f "$TARGET" Version)
readonly PREVIOUS_VERSION=$(dpkg-deb -f "$PREVIOUS" Version)
dpkg --compare-versions "$TARGET_VERSION" gt "$PREVIOUS_VERSION" || fail 'Target version must be newer than previous artifact'
if [[ "$(dpkg-query -W -f='${Status}' maintainer 2>/dev/null || :)" == 'install ok installed' ]]; then
    fail 'Start from a disposable system without Maintainer installed'
fi
readonly TARGET_HASH=$(sha256sum -- "$TARGET")
readonly PREVIOUS_HASH=$(sha256sum -- "$PREVIOUS")
verify_artifacts() {
    [[ "$(sha256sum -- "$TARGET")" == "$TARGET_HASH" ]] || fail 'Target artifact changed during verification'
    [[ "$(sha256sum -- "$PREVIOUS")" == "$PREVIOUS_HASH" ]] || fail 'Previous artifact changed during verification'
}

readonly APP_ID=io.github.maintainerapp.Maintainer
readonly SERVICE=io.github.maintainerapp.Helper
readonly WORK=$(mktemp -d /tmp/maintainer-package-test.XXXXXXXX)
SMOKE_USER="maintainer-smoke-${WORK##*.}"
SMOKE_USER=${SMOKE_USER,,}
CREATED_USER=0
cleanup() {
    local result=$?
    trap - EXIT
    if [[ "$CREATED_USER" == 1 ]]; then
        userdel --remove "$SMOKE_USER" >/dev/null 2>&1 || { printf 'FAIL: Could not remove smoke account\n' >&2; result=1; }
    fi
    rm -rf -- "$WORK"
    exit "$result"
}
trap cleanup EXIT
getent passwd "$SMOKE_USER" >/dev/null && fail 'Smoke account unexpectedly exists'
useradd --system --user-group --create-home --home-dir "$WORK/home" --shell /usr/sbin/nologin "$SMOKE_USER"
CREATED_USER=1
chmod 0755 "$WORK"

# Reuse the fixture-tested partial ordering and timestamp validation.
install -m644 "$(dirname -- "${BASH_SOURCE[0]}")/measure-performance.py" "$WORK/measurement.py"
cat > "$WORK/probe.py" <<'PY'
import os
import signal
import subprocess
import sys
import time

from measurement import await_ready

# The GUI must emit independent semantic milestones through this inherited pipe.
# Application diagnostics are intentionally discarded rather than logged.
read_fd, write_fd = os.pipe()
os.set_blocking(read_fd, False)
environment = os.environ.copy()
environment['MAINTAINER_PROBE_FD'] = str(write_fd)
started_ns = time.monotonic_ns()
process = subprocess.Popen(['/usr/bin/maintainer'], env=environment,
                           pass_fds=(write_fd,), stdin=subprocess.DEVNULL,
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                           start_new_session=True)
os.close(write_fd)
try:
    await_ready(process, read_fd, started_ns)
    # Check liveness after readiness, not merely successful spawning.
    time.sleep(0.25)
    if process.poll() is not None:
        raise RuntimeError('GUI exited immediately after readiness')
except Exception:
    print('FAIL: Installed GUI semantic readiness verification failed', file=sys.stderr)
    sys.exit(1)
finally:
    os.close(read_fd)
    if process.poll() is None:
        os.killpg(process.pid, signal.SIGTERM)
        try:
            process.wait(timeout=2)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGKILL)
            process.wait(timeout=2)
print('PASS: Installed unprivileged GUI mapped, usable and showing first data')
PY
chmod 0644 "$WORK/probe.py"

readonly -a FILES=(
    /usr/bin/maintainer
    /usr/libexec/maintainer-helper
    "/usr/share/applications/$APP_ID.desktop"
    "/usr/share/metainfo/$APP_ID.metainfo.xml"
    "/usr/share/glib-2.0/schemas/$APP_ID.gschema.xml"
    "/usr/share/dbus-1/system-services/$SERVICE.service"
    "/usr/share/dbus-1/system.d/$SERVICE.conf"
    /lib/systemd/system/maintainer-helper.service
    /usr/share/polkit-1/actions/io.github.maintainerapp.policy
    "/usr/share/icons/hicolor/scalable/apps/$APP_ID.svg"
    "/usr/share/icons/hicolor/symbolic/apps/$APP_ID-symbolic.svg"
    /usr/share/maintainer/styles/light.css
    /usr/share/maintainer/styles/dark.css
)
readonly -a OWNER_FILES=(
    /usr/libexec/maintainer-apt-backend.py
    /usr/share/dbus-1/system-services/io.github.maintainerapp.PackageOwner.service
    /usr/share/dbus-1/system.d/io.github.maintainerapp.PackageOwner.conf
    /lib/systemd/system/maintainer-package-owner.service
)
verify_payload_file() {
    local expected=$1 installed=$2
    [[ -f "$expected" && ! -L "$expected" ]] || fail 'Required regular file missing from tested archive'
    cmp --silent -- "$expected" "$installed" || fail 'Installed file bytes differ from tested archive'
}
verify_installed() {
    local expected_version=$1 expected_archive=$2 path mode
    local extracted
    extracted=$(mktemp -d "$WORK/payload.XXXXXXXX")
    dpkg-deb --extract "$expected_archive" "$extracted"
    [[ "$(dpkg-query -W -f='${Status}' maintainer)" == 'install ok installed' ]] || fail 'Package is not configured'
    [[ "$(dpkg-query -W -f='${Version}' maintainer)" == "$expected_version" ]] || fail 'Installed package version mismatch'
    local -a required=("${FILES[@]}")
    if [[ "$expected_archive" == "$TARGET" ]]; then
        required+=("${OWNER_FILES[@]}")
    fi
    for path in "${required[@]}"; do
        [[ -f "$path" && ! -L "$path" ]] || fail 'Required installed regular file missing'
        [[ "$(dpkg-query -S "$path")" == "maintainer: $path" ]] || fail 'Installed file is not owned by the package'
        mode=644
        [[ "$path" == /usr/bin/maintainer || "$path" == /usr/libexec/maintainer-helper || "$path" == /usr/libexec/maintainer-apt-backend.py ]] && mode=755
        [[ "$(stat -c '%u:%g:%a' -- "$path")" == "0:0:$mode" ]] || fail 'Installed file ownership or permissions are incorrect'
        verify_payload_file "$extracted$path" "$path"
    done
    [[ -d /usr/share/maintainer ]] || fail 'Application data directory missing'
    desktop-file-validate "/usr/share/applications/$APP_ID.desktop"
    appstreamcli validate --no-net "/usr/share/metainfo/$APP_ID.metainfo.xml"
    glib-compile-schemas --strict --dry-run /usr/share/glib-2.0/schemas
    [[ -f /usr/share/glib-2.0/schemas/gschemas.compiled ]] || fail 'Installed schema cache missing'
    gsettings list-schemas | grep -Fx "$APP_ID" >/dev/null || fail 'Installed GSettings schema is unavailable'
}

apt-get install --yes --no-install-recommends "$PREVIOUS"
verify_installed "$PREVIOUS_VERSION" "$PREVIOUS"
verify_artifacts
pass 'Previous version installed and metadata validated'
# Activate a harmless method so upgrade exercises a genuinely loaded old helper.
busctl --system --timeout=10 call "$SERVICE" /io/github/maintainerapp/Helper org.freedesktop.DBus.Peer Ping
OLD_OWNER=$(busctl --system call org.freedesktop.DBus /org/freedesktop/DBus org.freedesktop.DBus GetNameOwner s "$SERVICE")
OLD_PID=$(systemctl show maintainer-helper.service --property=MainPID --value)
[[ "$OLD_PID" =~ ^[1-9][0-9]*$ ]] || fail 'Previous helper did not activate through systemd'
OLD_START=$(python3 -c 'import pathlib,sys; print(pathlib.Path("/proc/"+sys.argv[1]+"/stat").read_text().rpartition(") ")[2].split()[19])' "$OLD_PID")
[[ "$(sha256sum "/proc/$OLD_PID/exe" | cut -d ' ' -f1)" == "$(sha256sum /usr/libexec/maintainer-helper | cut -d ' ' -f1)" ]] || fail 'Previous helper loaded binary differs from installed binary'
apt-get install --yes --no-install-recommends "$TARGET"
verify_installed "$TARGET_VERSION" "$TARGET"
verify_artifacts
if [[ -e "/proc/$OLD_PID/stat" ]]; then
    remaining_start=$(python3 -c 'import pathlib,sys; print(pathlib.Path("/proc/"+sys.argv[1]+"/stat").read_text().rpartition(") ")[2].split()[19])' "$OLD_PID")
    [[ "$remaining_start" != "$OLD_START" ]] || fail 'Old helper remains alive after upgrade'
fi
busctl --system --timeout=10 call "$SERVICE" /io/github/maintainerapp/Helper org.freedesktop.DBus.Peer Ping
NEW_OWNER=$(busctl --system call org.freedesktop.DBus /org/freedesktop/DBus org.freedesktop.DBus GetNameOwner s "$SERVICE")
[[ "$NEW_OWNER" != "$OLD_OWNER" ]] || fail 'Helper bus owner did not change after upgrade'
NEW_PID=$(systemctl show maintainer-helper.service --property=MainPID --value)
[[ "$NEW_PID" =~ ^[1-9][0-9]*$ ]] || fail 'Upgraded helper failed systemd activation'
[[ "$(sha256sum "/proc/$NEW_PID/exe" | cut -d ' ' -f1)" == "$(sha256sum /usr/libexec/maintainer-helper | cut -d ' ' -f1)" ]] || fail 'Upgraded helper is not running the installed binary'
pass 'Previously loaded helper exited naturally and upgraded binary owns the bus'
root_status=0
XDG_DATA_HOME="$WORK/root-data" /usr/bin/maintainer >"$WORK/root-output" 2>&1 || root_status=$?
[[ "$root_status" == 1 && ! -e "$WORK/root-data" ]] || fail 'Installed GUI did not refuse root before persistence'
grep -q 'must run as a normal user' "$WORK/root-output" || fail 'Installed GUI root-refusal explanation missing'
pass 'Installed GUI refuses root before creating application data'
runuser -u "$SMOKE_USER" -- dbus-run-session -- xvfb-run -a python3 "$WORK/probe.py"
verify_artifacts
pass 'Upgrade and installed semantic readiness verified'

dpkg --remove maintainer
for path in "${FILES[@]}" "${OWNER_FILES[@]}"; do
    [[ ! -e "$path" && ! -L "$path" ]] || fail 'Package file remains after removal'
done
dpkg --purge maintainer
[[ ! -e /var/lib/maintainer ]] || fail 'Application data remains after purge'
verify_artifacts
pass 'Install, upgrade, unprivileged readiness, removal and purge passed in this environment'
printf 'Not verified by this smoke: distribution matrix, interactive authorization or reboot behavior.\n'

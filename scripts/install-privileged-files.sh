#!/usr/bin/env bash
#
# Install the D-Bus bus policy, Polkit policy, and systemd unit from the source
# tree, for testing privileged actions without building and installing a .deb.
#
# The .deb installs these files itself, so this script exists only for working
# against a source checkout. It does not install the binaries; the helper is
# expected at /usr/libexec/maintainer-helper.
#
# Usage:
#   sudo bash scripts/install-privileged-files.sh
#   sudo bash scripts/install-privileged-files.sh --uninstall

set -euo pipefail

REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

BUS_POLICY_SRC="${REPO_DIR}/data/dbus/io.github.maintainerapp.Helper.conf"
POLKIT_SRC="${REPO_DIR}/data/polkit/io.github.maintainerapp.policy"
UNIT_SRC="${REPO_DIR}/data/systemd/maintainer-helper.service"
OWNER_POLICY_SRC="${REPO_DIR}/data/dbus/io.github.maintainerapp.PackageOwner.conf"
OWNER_ACTIVATION_SRC="${REPO_DIR}/data/dbus/io.github.maintainerapp.PackageOwner.service"
OWNER_UNIT_SRC="${REPO_DIR}/data/systemd/maintainer-package-owner.service"

BUS_POLICY_DEST="/usr/share/dbus-1/system.d/io.github.maintainerapp.Helper.conf"
POLKIT_DEST="/usr/share/polkit-1/actions/io.github.maintainerapp.policy"
UNIT_DEST="/lib/systemd/system/maintainer-helper.service"
OWNER_POLICY_DEST="/usr/share/dbus-1/system.d/io.github.maintainerapp.PackageOwner.conf"
OWNER_ACTIVATION_DEST="/usr/share/dbus-1/system-services/io.github.maintainerapp.PackageOwner.service"
OWNER_UNIT_DEST="/lib/systemd/system/maintainer-package-owner.service"

if [[ "${EUID}" -ne 0 ]]; then
    echo "This script writes to system directories and must run as root." >&2
    echo "Run: sudo bash scripts/install-privileged-files.sh" >&2
    exit 1
fi

reload_services() {
    # Reload rather than restart the bus: restarting the system bus disrupts
    # every connected service.
    systemctl reload dbus.service
    systemctl daemon-reload
    systemctl restart polkit.service
}

if [[ "${1:-}" == "--uninstall" ]]; then
    rm -f "${BUS_POLICY_DEST}" "${POLKIT_DEST}" "${UNIT_DEST}" "${OWNER_POLICY_DEST}" "${OWNER_ACTIVATION_DEST}" "${OWNER_UNIT_DEST}"
    reload_services
    echo "Removed the bus policy, Polkit policy, and unit."
    exit 0
fi

for src in "${BUS_POLICY_SRC}" "${POLKIT_SRC}" "${UNIT_SRC}" "${OWNER_POLICY_SRC}" "${OWNER_ACTIVATION_SRC}" "${OWNER_UNIT_SRC}"; do
    if [[ ! -f "${src}" ]]; then
        echo "Missing source file: ${src}" >&2
        exit 1
    fi
done

install -Dm644 "${BUS_POLICY_SRC}" "${BUS_POLICY_DEST}"
install -Dm644 "${POLKIT_SRC}" "${POLKIT_DEST}"
install -Dm644 "${UNIT_SRC}" "${UNIT_DEST}"
install -Dm644 "${OWNER_POLICY_SRC}" "${OWNER_POLICY_DEST}"
install -Dm644 "${OWNER_ACTIVATION_SRC}" "${OWNER_ACTIVATION_DEST}"
install -Dm644 "${OWNER_UNIT_SRC}" "${OWNER_UNIT_DEST}"

reload_services

echo "Installed:"
echo "  ${BUS_POLICY_DEST}"
echo "  ${POLKIT_DEST}"
echo "  ${UNIT_DEST}"
echo "  ${OWNER_POLICY_DEST}"
echo "  ${OWNER_ACTIVATION_DEST}"
echo "  ${OWNER_UNIT_DEST}"
echo

if ! systemctl cat maintainer-helper.service >/dev/null 2>&1; then
    echo "WARNING: systemd does not recognize maintainer-helper.service." >&2
fi

if [[ ! -x /usr/libexec/maintainer-helper ]]; then
    echo "WARNING: /usr/libexec/maintainer-helper is missing, so activation" >&2
    echo "         will fail. Install the .deb, or copy the built helper there." >&2
fi

echo "Authorization required for each action:"
POLICY_ERRORS=0
for action in terminate-process vacuum-journal clean-package-cache refresh-smart remove-kernel install-updates; do
    result="$(pkaction --action-id "io.github.maintainerapp.${action}" --verbose 2>/dev/null \
        | sed -n 's/^[[:space:]]*implicit active:[[:space:]]*//p')"
    if [[ -z "${result}" ]]; then
        echo "  ERROR: action io.github.maintainerapp.${action} not found in installed policy" >&2
        POLICY_ERRORS=$((POLICY_ERRORS + 1))
    else
        printf '  %-22s %s\n' "${action}" "${result}"
    fi
done

# Verify install-updates has the exec.path annotation.
EXEC_PATH="$(pkaction --action-id "io.github.maintainerapp.install-updates" --verbose 2>/dev/null \
    | grep -o '/usr/libexec/maintainer-helper' || true)"
if [[ -z "${EXEC_PATH}" ]]; then
    echo "  ERROR: install-updates action is missing org.freedesktop.policykit.exec.path annotation" >&2
    POLICY_ERRORS=$((POLICY_ERRORS + 1))
fi

if [[ "${POLICY_ERRORS}" -gt 0 ]]; then
    echo "FATAL: ${POLICY_ERRORS} policy verification error(s) found." >&2
    exit 1
fi

if [[ ! -f /usr/libexec/maintainer-apt-backend.py ]]; then
    echo "WARNING: fixed native APT backend is missing; package transactions require the .deb."
fi

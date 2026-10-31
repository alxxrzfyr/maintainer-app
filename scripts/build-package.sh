#!/usr/bin/env bash
# Package already-built release binaries; never rebuild the candidate here.
set -euo pipefail
umask 022
PKG_NAME="maintainer"
PKG_VERSION=$(cargo metadata --locked --no-deps --format-version 1 | python3 -c 'import json,sys; print(next(package["version"] for package in json.load(sys.stdin)["packages"] if package["name"] == "maintainer-app"))')
# Read from dpkg rather than hardcoding, so the package metadata can
# never disagree with the architecture the binaries were built for.
PKG_ARCH="$(dpkg --print-architecture)"
if [ "$PKG_ARCH" != "${EXPECTED_DEB_ARCH:?Expected architecture required}" ]; then
  echo "Runner architecture $PKG_ARCH does not match expected ${EXPECTED_DEB_ARCH:?Expected architecture required}" >&2
  exit 1
fi
[[ $# -le 1 ]] || { echo 'Usage: build-package.sh [external-output-directory]' >&2; exit 1; }
PKG_SOURCE_DIR=$(pwd -P)
if [[ $# == 1 ]]; then
  PKG_OUTPUT_DIR=$(realpath -m -- "$1")
else
  PKG_OUTPUT_DIR=$(mktemp -d /tmp/maintainer-artifacts.XXXXXXXX)
fi
case "$PKG_OUTPUT_DIR/" in
  "$PKG_SOURCE_DIR/"*) echo 'Package output must be outside the source tree' >&2; exit 1 ;;
esac
mkdir -p -- "$PKG_OUTPUT_DIR"
PKG_OUTPUT="$PKG_OUTPUT_DIR/${PKG_NAME}_${PKG_VERSION}-1_${PKG_ARCH}.deb"
PKG_STAGE=$(mktemp -d "${TMPDIR:-/tmp}/maintainer-package.XXXXXXXX")
cleanup() { rm -rf -- "$PKG_STAGE"; }
trap cleanup EXIT
PKG_DIR="$PKG_STAGE/root"

mkdir -p "${PKG_DIR}/DEBIAN"
mkdir -p "${PKG_DIR}/usr/bin"
mkdir -p "${PKG_DIR}/usr/libexec"
mkdir -p "${PKG_DIR}/usr/share/applications"
mkdir -p "${PKG_DIR}/usr/share/metainfo"
mkdir -p "${PKG_DIR}/usr/share/glib-2.0/schemas"
mkdir -p "${PKG_DIR}/usr/share/dbus-1/system-services"
mkdir -p "${PKG_DIR}/usr/share/dbus-1/system.d"
mkdir -p "${PKG_DIR}/usr/share/polkit-1/actions"
mkdir -p "${PKG_DIR}/lib/systemd/system"
mkdir -p "${PKG_DIR}/usr/share/icons/hicolor/scalable/apps"
mkdir -p "${PKG_DIR}/usr/share/icons/hicolor/symbolic/apps"
mkdir -p "${PKG_DIR}/usr/share/maintainer/styles"

# Binaries
install -m755 target/release/maintainer-app "${PKG_DIR}/usr/bin/maintainer"
install -m755 target/release/maintainer-helper "${PKG_DIR}/usr/libexec/maintainer-helper"

# Fixed native APT backend (python3-apt is the system package-manager binding).
install -m755 scripts/maintainer-apt-backend.py "${PKG_DIR}/usr/libexec/maintainer-apt-backend.py"

# Data files
install -m644 data/io.github.maintainerapp.Maintainer.desktop \
  "${PKG_DIR}/usr/share/applications/io.github.maintainerapp.Maintainer.desktop"
install -m644 data/io.github.maintainerapp.Maintainer.metainfo.xml \
  "${PKG_DIR}/usr/share/metainfo/io.github.maintainerapp.Maintainer.metainfo.xml"
install -m644 data/io.github.maintainerapp.Maintainer.gschema.xml \
  "${PKG_DIR}/usr/share/glib-2.0/schemas/io.github.maintainerapp.Maintainer.gschema.xml"
install -m644 data/dbus/io.github.maintainerapp.Helper.service \
  "${PKG_DIR}/usr/share/dbus-1/system-services/io.github.maintainerapp.Helper.service"
install -m644 data/dbus/io.github.maintainerapp.Helper.conf \
  "${PKG_DIR}/usr/share/dbus-1/system.d/io.github.maintainerapp.Helper.conf"
install -m644 data/polkit/io.github.maintainerapp.policy \
  "${PKG_DIR}/usr/share/polkit-1/actions/io.github.maintainerapp.policy"
install -m644 data/systemd/maintainer-helper.service \
  "${PKG_DIR}/lib/systemd/system/maintainer-helper.service"
install -m644 data/icons/hicolor/scalable/apps/io.github.maintainerapp.Maintainer.svg \
  "${PKG_DIR}/usr/share/icons/hicolor/scalable/apps/io.github.maintainerapp.Maintainer.svg"
install -m644 data/icons/hicolor/symbolic/apps/io.github.maintainerapp.Maintainer-symbolic.svg \
  "${PKG_DIR}/usr/share/icons/hicolor/symbolic/apps/io.github.maintainerapp.Maintainer-symbolic.svg"
install -m644 data/styles/light.css "${PKG_DIR}/usr/share/maintainer/styles/light.css"
install -m644 data/styles/dark.css "${PKG_DIR}/usr/share/maintainer/styles/dark.css"

install -m644 data/dbus/io.github.maintainerapp.PackageOwner.service \
  "${PKG_DIR}/usr/share/dbus-1/system-services/io.github.maintainerapp.PackageOwner.service"
install -m644 data/dbus/io.github.maintainerapp.PackageOwner.conf \
  "${PKG_DIR}/usr/share/dbus-1/system.d/io.github.maintainerapp.PackageOwner.conf"
install -m644 data/systemd/maintainer-package-owner.service \
  "${PKG_DIR}/lib/systemd/system/maintainer-package-owner.service"

# DEBIAN control file
cat > "${PKG_DIR}/DEBIAN/control" <<EOF
Package: maintainer
Version: ${PKG_VERSION}-1
Architecture: ${PKG_ARCH}
Maintainer: Alex Berin <maintainer@maintainerapp.io>
Depends: python3, python3-apt, libgtk-4-1 (>= 4.6), libglib2.0-0 (>= 2.72), libsqlite3-0, polkitd | policykit-1, systemd
Recommends: packagekit, udisks2
Suggests: flatpak, fwupd
Section: admin
Priority: optional
Homepage: https://github.com/alxxrzfyr/maintainer-app
Description: System monitor and maintenance tool for Ubuntu-based Linux
 Maintainer is a modern, lightweight native desktop application for
 monitoring, maintaining, diagnosing, and understanding Ubuntu-based
 Linux computers.
 .
 The GUI always runs unprivileged. No permanent daemon. No telemetry.
EOF

# Maintainer scripts
install -m755 packaging/debian/preinst "${PKG_DIR}/DEBIAN/preinst"
install -m755 packaging/debian/postinst "${PKG_DIR}/DEBIAN/postinst"
install -m755 packaging/debian/postrm "${PKG_DIR}/DEBIAN/postrm"

# Build the package
fakeroot dpkg-deb --build "${PKG_DIR}" "$PKG_OUTPUT"

echo "Built: $PKG_OUTPUT"
dpkg-deb --info "$PKG_OUTPUT"

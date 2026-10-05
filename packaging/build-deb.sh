#!/bin/sh
# Build a .deb with dpkg-deb (no extra tooling). Reproducible: timestamps come
# from SOURCE_DATE_EPOCH (defaults to the last commit or a fixed value) and the
# build uses the committed Cargo.lock.
set -eu
cd "$(dirname "$0")/.."
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
ARCH=$(dpkg --print-architecture)
: "${SOURCE_DATE_EPOCH:=$(git log -1 --format=%ct 2>/dev/null || echo 1767225600)}"
export SOURCE_DATE_EPOCH

cargo build --release --locked

STAGE=target/deb/systemhealthcheck_${VERSION}_${ARCH}
rm -rf "$STAGE"
install -Dm755 target/release/SystemHealthCheck "$STAGE/usr/bin/SystemHealthCheck"
install -Dm644 data/io.github.systemhealthcheck.SystemHealthCheck.desktop "$STAGE/usr/share/applications/io.github.systemhealthcheck.SystemHealthCheck.desktop"
install -Dm644 data/io.github.systemhealthcheck.SystemHealthCheck.svg "$STAGE/usr/share/icons/hicolor/scalable/apps/io.github.systemhealthcheck.SystemHealthCheck.svg"
install -Dm644 data/io.github.systemhealthcheck.SystemHealthCheck.policy "$STAGE/usr/share/polkit-1/actions/io.github.systemhealthcheck.SystemHealthCheck.policy"
install -Dm644 README.md "$STAGE/usr/share/doc/systemhealthcheck/README.md"
mkdir -p "$STAGE/DEBIAN"
cat > "$STAGE/DEBIAN/control" <<CONTROL
Package: systemhealthcheck
Version: ${VERSION}
Architecture: ${ARCH}
Maintainer: Tanmay Srivastava <tanmay.srivastava891@gmail.com>
Section: admin
Priority: optional
Depends: libgtk-4-1 (>= 4.12), libadwaita-1-0 (>= 1.6), pkexec | policykit-1
Recommends: lshw, dmidecode, pciutils, usbutils, smartmontools, nvme-cli, lm-sensors, mesa-utils, upower, hdparm, iw, network-manager
Suggests: sysbench, stress-ng, linux-cpupower, htop
Description: Linux laptop diagnostic & performance center
 Native GTK4/libadwaita application that runs the checklist diagnostics
 (lshw, dmidecode, smartctl, nvme, sensors, upower, systemd-analyze, dmesg,
 lspci, lsusb, ...) through an allowlisted command registry, monitors the
 system live from /proc and /sys, scores health transparently, keeps a local
 history and exports reports. No telemetry; nothing leaves the machine.
CONTROL
find "$STAGE" -exec touch -h -d "@${SOURCE_DATE_EPOCH}" {} +
dpkg-deb --root-owner-group -Zxz --build "$STAGE" target/deb/
echo "Built: target/deb/systemhealthcheck_${VERSION}_${ARCH}.deb"

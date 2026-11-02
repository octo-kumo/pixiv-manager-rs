#!/usr/bin/env bash
# Package the Linux release artifacts.
#
#   dist/pixiv-manager-VERSION-x86_64-linux.tar.gz
#
# The tarball is portable: extract it anywhere and run the binaries. Linux
# system integration (desktop entry, icons, PATH) stays the job of
# ./install.sh, which is intentionally Linux/KDE specific.
set -euo pipefail

VERSION="${1:?usage: package-linux.sh VERSION [BINDIR]}"
BIN_DIR="${2:-target/release}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DIST="${ROOT}/dist"
NAME="pixiv-manager-${VERSION}-x86_64-linux"

cd "${ROOT}"
rm -rf "${DIST:?}/${NAME}"
mkdir -p "${DIST}/${NAME}"

install -m 0755 "${BIN_DIR}/pixiv-manager" "${DIST}/${NAME}/pixiv-manager"
install -m 0755 "${BIN_DIR}/pixiv-manager-cli" "${DIST}/${NAME}/pixiv-manager-cli"
cp README.md "${DIST}/${NAME}/"
[[ -f LICENSE ]] && cp LICENSE "${DIST}/${NAME}/"
cp assets/icons/pixiv-manager.png "${DIST}/${NAME}/pixiv-manager.png"
cp packaging/linux/pixiv-manager.desktop "${DIST}/${NAME}/"

tar -C "${DIST}" -czf "${DIST}/${NAME}.tar.gz" "${NAME}"
rm -rf "${DIST}/${NAME}"
echo "${DIST}/${NAME}.tar.gz"

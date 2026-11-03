#!/usr/bin/env bash
set -euo pipefail

# ── Pixiv Manager — Install Script ─────────────────────────────────────
# Builds the project, installs binaries to ~/.local/bin, and
# creates a KDE Plasma desktop entry.
#
# Usage: ./install.sh

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
BIN_DIR="${HOME}/.local/bin"
APPS_DIR="${HOME}/.local/share/applications"
ICONS_DIR="${HOME}/.local/share/icons/hicolor/256x256/apps"
BINARY_NAME="pixiv-manager"
CLI_BINARY_NAME="pixiv-manager-cli"

echo "==> Pixiv Manager — Install"
echo "    Source: ${SCRIPT_DIR}"
echo "    Target: ${BIN_DIR}"
echo ""

# ── 1. Build ────────────────────────────────────────────────────────
echo "==> Building release binary..."
cd "${SCRIPT_DIR}"
RUSTFLAGS="" cargo build --release

RELEASE_BIN="${SCRIPT_DIR}/target/release/${BINARY_NAME}"
CLI_BIN="${SCRIPT_DIR}/target/release/${CLI_BINARY_NAME}"

if [[ ! -f "${RELEASE_BIN}" ]]; then
    echo "ERROR: Build failed — ${RELEASE_BIN} not found"
    exit 1
fi

echo "    ✓ Built: $(du -h "${RELEASE_BIN}" | cut -f1)"

# ── 2. Install binaries ─────────────────────────────────────────────
echo "==> Installing binaries to ${BIN_DIR}..."
mkdir -p "${BIN_DIR}"

cp "${RELEASE_BIN}" "${BIN_DIR}/${BINARY_NAME}"
chmod +x "${BIN_DIR}/${BINARY_NAME}"

if [[ -f "${CLI_BIN}" ]]; then
    cp "${CLI_BIN}" "${BIN_DIR}/${CLI_BINARY_NAME}"
    chmod +x "${BIN_DIR}/${CLI_BINARY_NAME}"
    echo "    ✓ ${CLI_BINARY_NAME}"
fi

echo "    ✓ ${BINARY_NAME}"

# Check if ~/.local/bin is in PATH
if ! echo "${PATH}" | tr ':' '\n' | grep -qxF "${BIN_DIR}"; then
    echo ""
    echo "    ⚠  ${BIN_DIR} is not in your PATH."
    echo "       Add this to your ~/.bashrc or ~/.zshrc:"
    echo "       export PATH=\"\${HOME}/.local/bin:\${PATH}\""
fi

# ── 3. Desktop entry (KDE Plasma) ───────────────────────────────────
echo "==> Creating desktop entry..."
mkdir -p "${APPS_DIR}"

cat > "${APPS_DIR}/pixiv-manager.desktop" << EOF
[Desktop Entry]
Type=Application
Name=Pixiv Manager
Comment=Browse, search, and manage Pixiv illustrations
Exec=${BIN_DIR}/${BINARY_NAME}
Icon=pixiv-manager
Terminal=false
Categories=Graphics;Network;Qt;
StartupNotify=false
StartupWMClass=pixiv-manager
EOF

echo "    ✓ ${APPS_DIR}/pixiv-manager.desktop"

# ── 4. Icon ─────────────────────────────────────────────────────────
echo "==> Installing icon..."
mkdir -p "${ICONS_DIR}"

# The icon is a version-controlled asset shared with the Windows executable.
ICON_SRC="${SCRIPT_DIR}/assets/icons/pixiv-manager.png"
if [[ -f "${ICON_SRC}" ]]; then
    cp "${ICON_SRC}" "${ICONS_DIR}/pixiv-manager.png"
    echo "    ✓ ${ICONS_DIR}/pixiv-manager.png"
else
    echo "    ⚠  ${ICON_SRC} missing — no icon installed"
fi

# ── 5. Refresh KDE cache ────────────────────────────────────────────
echo "==> Refreshing desktop database..."
if command -v update-desktop-database &>/dev/null; then
    update-desktop-database "${APPS_DIR}" 2>/dev/null || true
    echo "    ✓ Desktop database updated"
fi

if command -v kbuildsycoca5 &>/dev/null; then
    kbuildsycoca5 --noincremental 2>/dev/null || true
    echo "    ✓ KDE service cache rebuilt"
elif command -v kbuildsycoca6 &>/dev/null; then
    kbuildsycoca6 --noincremental 2>/dev/null || true
    echo "    ✓ KDE 6 service cache rebuilt"
fi

echo ""
echo "════════════════════════════════════════════════════════════════"
echo "  ✓ Pixiv Manager installed!"
echo ""
echo "  Launch from:"
echo "    • Application menu → Graphics → Pixiv Manager"
echo "    • Terminal:    ${BINARY_NAME}"
echo "    • CLI:         ${CLI_BINARY_NAME}"
echo ""
echo "  On first launch, you'll be guided through onboarding:"
echo "    • Enter your Pixiv refresh token"
echo "    • Select a directory for storing illustrations"
echo "════════════════════════════════════════════════════════════════"
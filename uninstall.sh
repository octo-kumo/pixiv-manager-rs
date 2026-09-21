#!/usr/bin/env bash
set -euo pipefail

# ── Pixiv Manager — Uninstall Script ──────────────────────────────────

BIN_DIR="${HOME}/.local/bin"
APPS_DIR="${HOME}/.local/share/applications"
ICONS_DIR="${HOME}/.local/share/icons/hicolor/256x256/apps"

echo "==> Pixiv Manager — Uninstall"
echo ""

echo "==> Removing binaries..."
rm -f "${BIN_DIR}/pixiv-manager"
rm -f "${BIN_DIR}/pixiv-manager-cli"
echo "    ✓ Removed"

echo "==> Removing desktop entry..."
rm -f "${APPS_DIR}/pixiv-manager.desktop"
echo "    ✓ Removed"

echo "==> Removing icons..."
rm -f "${ICONS_DIR}/pixiv-manager.png"
rm -f "${ICONS_DIR}/pixiv-manager.svg"
echo "    ✓ Removed"

# Refresh KDE cache
if command -v update-desktop-database &>/dev/null; then
    update-desktop-database "${APPS_DIR}" 2>/dev/null || true
fi
if command -v kbuildsycoca6 &>/dev/null; then
    kbuildsycoca6 --noincremental 2>/dev/null || true
elif command -v kbuildsycoca5 &>/dev/null; then
    kbuildsycoca5 --noincremental 2>/dev/null || true
fi

echo ""
echo "✓ Pixiv Manager uninstalled."
echo "  Config and data at ~/.config/pixiv-manager/ are preserved."
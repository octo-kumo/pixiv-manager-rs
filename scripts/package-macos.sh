#!/usr/bin/env bash
# Package the macOS release artifacts.
#
#   dist/Pixiv-Manager-VERSION-universal.dmg          GUI .app, signed+notarized
#   dist/pixiv-manager-cli-VERSION-macos.tar.gz      CLI, separate tarball
#
# The GUI is a Universal 2 bundle (arm64 + x86_64). The CLI stays a plain
# executable tarball rather than being hidden inside the bundle, so it can be
# dropped into ~/.local/bin or Homebrew.
#
# Signing/notarization are opt-in via environment (never via the repository):
#   MACOS_SIGN_IDENTITY    e.g. "Developer ID Application: Name (TEAMID)"
#   MACOS_TEAM_ID          Apple team id
#   MACOS_NOTARY_APPLE_ID  Apple ID for notarytool
#   MACOS_NOTARY_PASSWORD  app-specific password
#
# This script is written for macOS runners and is not exercised on Linux; it is
# kept in the repository so the release pipeline stays declarative.
set -euo pipefail

VERSION="${1:?usage: package-macos.sh VERSION}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DIST="${ROOT}/dist"
APP_NAME="Pixiv Manager"
BUNDLE_ID="io.github.octo-kumo.pixiv-manager"
APP="${DIST}/${APP_NAME}.app"
DMG="${DIST}/Pixiv-Manager-${VERSION}-universal.dmg"

cd "${ROOT}"
rm -rf "${DIST}"
mkdir -p "${DIST}"

# ── 1. Universal 2 binaries ────────────────────────────────────────────
for target in aarch64-apple-darwin x86_64-apple-darwin; do
    rustup target add "${target}"
    cargo build --release --locked --target "${target}"
done

mkdir -p "${DIST}/universal"
for bin in pixiv-manager pixiv-manager-cli; do
    lipo -create \
        "target/aarch64-apple-darwin/release/${bin}" \
        "target/x86_64-apple-darwin/release/${bin}" \
        -output "${DIST}/universal/${bin}"
done

# ── 2. .app bundle ─────────────────────────────────────────────────────
mkdir -p "${APP}/Contents/MacOS" "${APP}/Contents/Resources"
cp "${DIST}/universal/pixiv-manager" "${APP}/Contents/MacOS/pixiv-manager"
cp assets/icons/pixiv-manager.icns "${APP}/Contents/Resources/icon.icns"

# Info.plist is generated from the committed template so the version can never
# drift from the release version computed once by the pipeline.
sed -e "s/@@VERSION@@/${VERSION}/g" -e "s/@@BUNDLE_ID@@/${BUNDLE_ID}/g" \
    packaging/macos/Info.plist.in > "${APP}/Contents/Info.plist"

# ── 3. Sign (hardened runtime) ─────────────────────────────────────────
if [[ -n "${MACOS_SIGN_IDENTITY:-}" ]]; then
    echo "==> Signing with ${MACOS_SIGN_IDENTITY}"
    codesign --force --deep --options runtime --timestamp \
        --entitlements packaging/macos/entitlements.plist \
        --sign "${MACOS_SIGN_IDENTITY}" "${APP}"
    codesign --verify --deep --strict --verbose=2 "${APP}"
else
    echo "!!  MACOS_SIGN_IDENTITY not set — bundle is UNSIGNED and will not pass Gatekeeper" >&2
fi

# ── 4. DMG ─────────────────────────────────────────────────────────────
echo "==> Creating ${DMG}"
hdiutil create -volname "${APP_NAME}" -srcfolder "${APP}" -ov -format UDZO "${DMG}"

# ── 5. Notarize + staple ───────────────────────────────────────────────
if [[ -n "${MACOS_SIGN_IDENTITY:-}" && -n "${MACOS_NOTARY_APPLE_ID:-}" && -n "${MACOS_NOTARY_PASSWORD:-}" ]]; then
    echo "==> Submitting for notarization"
    xcrun notarytool submit "${DMG}" \
        --apple-id "${MACOS_NOTARY_APPLE_ID}" \
        --team-id "${MACOS_TEAM_ID:?MACOS_TEAM_ID required for notarization}" \
        --password "${MACOS_NOTARY_PASSWORD}" \
        --wait
    xcrun stapler staple "${DMG}"
    xcrun stapler validate "${DMG}"
    spctl --assess --type open --context context:primary-signature -v "${DMG}"
else
    echo "!!  notarization credentials not set — DMG is NOT notarized" >&2
fi

# ── 6. CLI tarball ─────────────────────────────────────────────────────
CLI_NAME="pixiv-manager-cli-${VERSION}-macos"
mkdir -p "${DIST}/${CLI_NAME}"
cp "${DIST}/universal/pixiv-manager-cli" "${DIST}/${CLI_NAME}/"
cp README.md "${DIST}/${CLI_NAME}/"
tar -C "${DIST}" -czf "${DIST}/${CLI_NAME}.tar.gz" "${CLI_NAME}"
rm -rf "${DIST}/${CLI_NAME}" "${DIST}/universal"

echo "==> Artifacts"
ls -l "${DIST}"

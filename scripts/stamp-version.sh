#!/usr/bin/env bash
# Stamp a release version into the workspace and refresh Cargo.lock.
#
# The release pipeline computes the version ONCE (prepare-version) and every
# platform job calls this script with that value, so all artifacts advertise the
# same version and are built from the same commit + lockfile. `cargo build
# --locked` is used afterwards, which is why the lockfile must be refreshed in
# the same step: Cargo.lock records the workspace members' versions.
set -euo pipefail

VERSION="${1:?usage: stamp-version.sh X.Y.Z}"
if [[ ! "${VERSION}" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    echo "error: '${VERSION}' is not a plain MAJOR.MINOR.PATCH version" >&2
    exit 1
fi

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "${ROOT}"

# The first `version = ` in Cargo.toml is [workspace.package].version.
sed -i.bak -E '0,/^version = .*/s//version = "'"${VERSION}"'"/' Cargo.toml
rm -f Cargo.toml.bak
grep -q "version = \"${VERSION}\"" Cargo.toml || {
    echo "error: failed to stamp Cargo.toml" >&2
    exit 1
}

# Re-resolve only the workspace members; dependencies stay exactly as locked.
cargo update -p pixiv-manager -p pixivj-rs

# Prove the lockfile is consistent with the stamped manifest.
cargo metadata --locked --format-version 1 --no-deps > /dev/null

echo "stamped version ${VERSION}"

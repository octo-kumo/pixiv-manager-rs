//! Embeds Windows executable resources: icon, VERSIONINFO and manifest.
//!
//! Skipped for non-Windows targets, and for cross `cargo check` runs without a
//! resource compiler (`PIXIV_MANAGER_SKIP_WINRESOURCES=1`).

use std::env;

const COMPANY: &str = "Pixiv Manager contributors";

fn main() {
    println!("cargo:rerun-if-changed=../assets/windows/app.manifest");
    println!("cargo:rerun-if-changed=../assets/icons/pixiv-manager.ico");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=PIXIV_MANAGER_SKIP_WINRESOURCES");

    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    if env::var_os("PIXIV_MANAGER_SKIP_WINRESOURCES").is_some() {
        println!("cargo:warning=skipping Windows resources");
        return;
    }

    let manifest_dir = env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR");
    let assets = std::path::Path::new(&manifest_dir).join("../assets");
    // The path is embedded in a .rc file, where backslashes are escape
    // characters, so it has to be an absolute path with forward slashes.
    let icon = assets
        .join("icons/pixiv-manager.ico")
        .to_string_lossy()
        .replace('\\', "/");
    let manifest = std::fs::read_to_string(assets.join("windows/app.manifest"))
        .expect("assets/windows/app.manifest is missing");
    let version = env::var("CARGO_PKG_VERSION").expect("cargo sets CARGO_PKG_VERSION");

    embed(
        "pixiv-manager",
        "Pixiv Manager",
        Some(&icon),
        &manifest,
        &version,
    );
    embed(
        "pixiv-manager-cli",
        "Pixiv Manager command-line interface",
        None,
        &manifest,
        &version,
    );
}

fn embed(binary: &str, description: &str, icon: Option<&str>, manifest: &str, version: &str) {
    let mut res = winresource::WindowsResource::new();
    if let Some(icon) = icon {
        res.set_icon(icon);
    }
    res.set_manifest(manifest)
        .set("ProductName", "Pixiv Manager")
        .set("FileDescription", description)
        .set("OriginalFilename", &format!("{binary}.exe"))
        .set("InternalName", binary)
        .set("CompanyName", COMPANY)
        .set("LegalCopyright", &format!("(c) {COMPANY} - MIT licensed"))
        .set("ProductVersion", version)
        .set("FileVersion", version)
        .set_version_info(
            winresource::VersionInfo::FILEVERSION,
            version_words(version),
        )
        .set_version_info(
            winresource::VersionInfo::PRODUCTVERSION,
            version_words(version),
        )
        .set_version_info(winresource::VersionInfo::FILEOS, 0x0004_0004)
        .set_version_info(winresource::VersionInfo::FILETYPE, 0x0000_0001);
    res.compile_for(binary)
        .unwrap_or_else(|e| panic!("failed to embed resources into {binary}: {e}"));
}

/// `MAJOR.MINOR.PATCH` packed into the four `u16` words VERSIONINFO uses.
fn version_words(version: &str) -> u64 {
    let core = version.split(['-', '+']).next().unwrap_or(version);
    let mut parts = core.split('.').map(|p| p.parse::<u64>().unwrap_or(0));
    let major = parts.next().unwrap_or(0) & 0xffff;
    let minor = parts.next().unwrap_or(0) & 0xffff;
    let patch = parts.next().unwrap_or(0) & 0xffff;
    (major << 48) | (minor << 32) | (patch << 16)
}

#[cfg(test)]
mod tests {
    use super::version_words;

    #[test]
    fn packs_versions() {
        assert_eq!(version_words("1.2.3"), (1 << 48) | (2 << 32) | (3 << 16));
        assert_eq!(version_words("0.1.0-beta.2"), version_words("0.1.0"));
        assert_eq!(version_words("2.5"), 2 << 48 | 5 << 32);
        assert_eq!(version_words("junk"), 0);
    }
}

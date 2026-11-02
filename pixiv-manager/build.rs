//! Build script.
//!
//! On Windows targets this embeds the executable resources — application
//! icon, `VERSIONINFO` metadata and the application manifest — straight into
//! the binaries, from version-controlled assets under
//! `assets/`:
//!
//! * `pixiv-manager.exe` gets the icon, the manifest and product metadata.
//! * `pixiv-manager-cli.exe` gets the same manifest (DPI/long paths) and its
//!   own `OriginalFilename`/description, so the two executables are
//!   distinguishable in Explorer and in antivirus consoles.
//!
//! Nothing is generated at install time: an installer that synthesises icons
//! produces inconsistent artifacts and looks like a packer to security tools.
//!
//! Cross-compiling a `*-windows-*` target from a host without a Windows
//! resource compiler (`rc.exe`/`windres`) is still possible for
//! `cargo check`-style validation by setting
//! `PIXIV_MANAGER_SKIP_WINRESOURCES=1`.

use std::env;

/// Stable publisher identity used in the Windows `VERSIONINFO` block.
const COMPANY: &str = "Pixiv Manager contributors";
/// SPDX license identifier of the project (see the README / LICENSE).
const LICENSE: &str = "MIT";

fn main() {
    // Assets live at the workspace root (shared with the packaging scripts and
    // the Linux installer), one level above this crate.
    println!("cargo:rerun-if-changed=../assets/windows/app.manifest");
    println!("cargo:rerun-if-changed=../assets/icons/pixiv-manager.ico");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=PIXIV_MANAGER_SKIP_WINRESOURCES");

    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    if env::var_os("PIXIV_MANAGER_SKIP_WINRESOURCES").is_some() {
        println!("cargo:warning=skipping Windows resources (PIXIV_MANAGER_SKIP_WINRESOURCES)");
        return;
    }

    // The resource compiler is not guaranteed to run with the package root as
    // its working directory (windres does not), so hand it absolute paths.
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR");
    let assets = std::path::Path::new(&manifest_dir).join("../assets");
    // Forward slashes only: the path is embedded in a `.rc` file where
    // backslashes are escape characters (`\p` is not a valid escape), so a
    // native `C:\...` path makes windres fail with "Invalid argument".
    let icon = assets
        .join("icons/pixiv-manager.ico")
        .to_string_lossy()
        .replace('\\', "/");

    let version = env::var("CARGO_PKG_VERSION").expect("cargo sets CARGO_PKG_VERSION");
    let numeric = version_words(&version);
    let manifest = std::fs::read_to_string(assets.join("windows/app.manifest"))
        .expect("assets/windows/app.manifest is missing");

    // GUI: icon + manifest + product metadata.
    let mut gui = winresource::WindowsResource::new();
    gui.set_icon(&icon)
        .set_manifest(&manifest)
        .set("ProductName", "Pixiv Manager")
        .set("FileDescription", "Pixiv Manager")
        .set("OriginalFilename", "pixiv-manager.exe")
        .set("InternalName", "pixiv-manager")
        .set("CompanyName", COMPANY)
        .set(
            "LegalCopyright",
            &format!("© {COMPANY} — {LICENSE} licensed"),
        )
        .set(
            "Comments",
            "Browse, download and manage Pixiv illustrations",
        )
        .set("ProductVersion", &version)
        .set("FileVersion", &version)
        .set_version_info(winresource::VersionInfo::FILEVERSION, numeric)
        .set_version_info(winresource::VersionInfo::PRODUCTVERSION, numeric)
        // VOS_NT_WINDOWS32 | VFT_APP: a plain 32/64-bit Windows GUI application.
        .set_version_info(winresource::VersionInfo::FILEOS, 0x0004_0004)
        .set_version_info(winresource::VersionInfo::FILETYPE, 0x0000_0001);
    gui.compile_for("pixiv-manager")
        .expect("failed to embed Windows resources into pixiv-manager.exe");

    // CLI: same manifest, its own identity (no window icon needed).
    let mut cli = winresource::WindowsResource::new();
    cli.set_manifest(&manifest)
        .set("ProductName", "Pixiv Manager")
        .set("FileDescription", "Pixiv Manager command-line interface")
        .set("OriginalFilename", "pixiv-manager-cli.exe")
        .set("InternalName", "pixiv-manager-cli")
        .set("CompanyName", COMPANY)
        .set(
            "LegalCopyright",
            &format!("© {COMPANY} — {LICENSE} licensed"),
        )
        .set(
            "Comments",
            "Browse, download and manage Pixiv illustrations",
        )
        .set("ProductVersion", &version)
        .set("FileVersion", &version)
        .set_version_info(winresource::VersionInfo::FILEVERSION, numeric)
        .set_version_info(winresource::VersionInfo::PRODUCTVERSION, numeric)
        .set_version_info(winresource::VersionInfo::FILEOS, 0x0004_0004)
        .set_version_info(winresource::VersionInfo::FILETYPE, 0x0000_0001);
    cli.compile_for("pixiv-manager-cli")
        .expect("failed to embed Windows resources into pixiv-manager-cli.exe");
}

/// Pack `MAJOR.MINOR.PATCH` into the four `u16` words `VERSIONINFO` uses.
///
/// A pre-release suffix (`1.2.3-beta.1`) only affects the string fields; the
/// numeric fields use the leading numbers, with the 4th word left at 0.
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
    fn packs_three_component_versions() {
        assert_eq!(version_words("1.2.3"), (1 << 48) | (2 << 32) | (3 << 16));
    }

    #[test]
    fn tolerates_prerelease_and_short_versions() {
        assert_eq!(version_words("0.1.0-beta.2"), version_words("0.1.0"));
        assert_eq!(version_words("2.5"), 2 << 48 | 5 << 32);
        assert_eq!(version_words("junk"), 0);
    }
}

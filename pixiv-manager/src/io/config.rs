//! Config persistence (shared by the GUI and the CLI).
//!
//! Lives here (not in `app::update`) so the CLI binary does not reach into the
//! GUI reducer for plain file IO, and so config-format changes affect both
//! front-ends through one code path.

use crate::app::state::Settings;

/// Load config from disk.
pub fn load_config() -> Option<Settings> {
    let path = config_path()?;
    #[cfg(feature = "profiling")]
    let start = std::time::Instant::now();
    let content = std::fs::read_to_string(&path).ok()?;
    #[cfg(feature = "profiling")]
    crate::profiling::tracing::record_file(
        crate::profiling::tracing::FileOp::Read,
        path.display().to_string(),
        content.len(),
        start.elapsed(),
    );
    toml::from_str(&content).ok()
}

/// Save config to disk.
///
/// The config holds the OAuth refresh token (≈ account password), so on unix
/// it is written with owner-only `0600` permissions and any legacy (world-
/// readable) config is re-permissioned on every save and at boot.
pub fn save_config(settings: &Settings) {
    if let Some(path) = config_path() {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(content) = toml::to_string_pretty(settings) {
            write_config(&path, content.as_bytes());
        }
    }
}

/// Restrict an existing config file to owner-only (`0600`). Called at boot so
/// configs written by older versions (which used the default umask, e.g.
/// `0644`) stop exposing the refresh token to other local users.
pub fn fix_config_permissions() {
    if let Some(path) = config_path() {
        if path.exists() {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
            }
        }
    }
}

/// Write the config with owner-only permissions.
fn write_config(path: &std::path::Path, bytes: &[u8]) {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true).mode(0o600);
        if let Ok(mut f) = opts.open(path) {
            let _ = f.write_all(bytes);
        }
    }
    #[cfg(not(unix))]
    {
        let _ = std::fs::write(path, bytes);
    }
}

pub fn config_path() -> Option<std::path::PathBuf> {
    // Tests set this to a temp dir so they never touch the real config.
    if let Ok(dir) = std::env::var("PIXIV_MANAGER_CONFIG_DIR") {
        if !dir.is_empty() {
            return Some(std::path::PathBuf::from(dir).join("config.toml"));
        }
    }
    // Test builds (feature `test-util`, enabled by the dev-dependency) MUST
    // never read or write the real user config — several onboarding tests
    // persist a fake token via `save_config`, which used to clobber
    // `~/.config/pixiv-manager/config.toml` and invalidate the user's saved
    // login. The CLI tests redirect the config via `XDG_CONFIG_HOME` (which
    // `dirs::config_dir()` honours below); everything else gets a per-process
    // temp dir instead, so `cargo test` is completely side-effect free.
    #[cfg(feature = "test-util")]
    if std::env::var("XDG_CONFIG_HOME")
        .map(|d| !d.is_empty())
        .unwrap_or(false)
    {
        // fall through to dirs::config_dir()
    } else {
        return Some(
            std::env::temp_dir()
                .join(format!("pixiv-manager-test-{}", std::process::id()))
                .join("config.toml"),
        );
    }
    dirs::config_dir().map(|p| p.join("pixiv-manager").join("config.toml"))
}

/// Where the persistent on-disk image cache lives.
pub fn cache_dir() -> std::path::PathBuf {
    dirs::data_dir()
        .map(|d| d.join("pixiv-manager").join("cache"))
        .unwrap_or_else(|| std::env::temp_dir().join("pixiv-manager-cache"))
}

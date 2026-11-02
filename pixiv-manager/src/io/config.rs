//! Config persistence (shared by the GUI and the CLI).
//!
//! Lives here (not in `app::update`) so the CLI binary does not reach into the
//! GUI reducer for plain file IO, and so config-format changes affect both
//! front-ends through one code path.
//!
//! The on-disk location comes from [`crate::io::paths::AppPaths`]; this module
//! never looks at platform directories itself.

use crate::app::state::Settings;
use crate::io::paths::AppPaths;

/// Load config from disk.
pub fn load_config() -> Option<Settings> {
    let path = config_path()?;
    let content = crate::io::read_file(&path)?;
    toml::from_str(&content).ok()
}

/// Save config to disk. The file holds the OAuth refresh token, so it is
/// written owner-only on Unix.
pub fn save_config(settings: &Settings) {
    if let Some(path) = config_path() {
        if let Ok(content) = toml::to_string_pretty(settings) {
            if let Err(e) = crate::io::write_atomic_private(&path, content.as_bytes()) {
                log::warn!("[config] save failed: {e}");
            }
        }
    }
}

/// Restrict an existing config file to owner-only (`0600`). Called at boot so
/// configs written by older versions (which used the default umask, e.g.
/// `0644`) stop exposing the refresh token to other local users.
pub fn fix_config_permissions() {
    if let Some(path) = config_path() {
        if !path.exists() {
            return;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
        #[cfg(not(unix))]
        let _ = &path;
    }
}

/// The active config file (`<config dir>/config.toml`).
pub fn config_path() -> Option<std::path::PathBuf> {
    Some(AppPaths::discover().config_file())
}

/// Where the persistent on-disk image cache lives.
///
/// This is the OS *cache* directory: the bytes are re-downloadable, so they do
/// not belong in the data directory that backup tools and the library live in.
/// [`crate::image::cache`] migrates any pre-existing
/// `<data>/pixiv-manager/cache` contents on first start.
pub fn cache_dir() -> std::path::PathBuf {
    AppPaths::discover().image_cache_dir()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_path_is_the_path_layer_config_file() {
        assert_eq!(config_path(), Some(AppPaths::discover().config_file()));
    }

    #[test]
    fn cache_dir_lives_under_the_cache_directory() {
        let paths = AppPaths::discover();
        assert_eq!(cache_dir(), paths.image_cache_dir());
        assert!(cache_dir().starts_with(&paths.cache_dir));
    }
}

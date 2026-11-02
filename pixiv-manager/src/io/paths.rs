//! Application directory layout.
//!
//! Every application path is resolved here; nothing else calls `dirs::*`.
//!
//! Config, persistent data and disposable cache are always three distinct
//! directories, including on the platforms whose OS directories coincide:
//! Windows puts local data and cache both under `%LOCALAPPDATA%`, macOS puts
//! config and local data both under `Application Support`.
//!
//! `PIXIV_MANAGER_CONFIG_DIR` overrides all three with one root (tests,
//! portable installs).

use std::path::{Path, PathBuf};

/// Directories the application owns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppPaths {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub cache_dir: PathBuf,
}

impl AppPaths {
    /// Resolve paths, honouring `PIXIV_MANAGER_CONFIG_DIR` first.
    pub fn discover() -> Self {
        if let Some(paths) = Self::from_override(std::env::var_os(OVERRIDE_ENV)) {
            return paths;
        }
        #[cfg(feature = "test-util")]
        {
            Self::rooted(
                std::env::temp_dir().join(format!("pixiv-manager-test-{}", std::process::id())),
            )
        }
        #[cfg(not(feature = "test-util"))]
        {
            Self::platform_defaults()
        }
    }

    /// One root for config, data and cache (`<root>/cache`).
    pub fn rooted(root: PathBuf) -> Self {
        Self {
            cache_dir: root.join("cache"),
            config_dir: root.clone(),
            data_dir: root,
        }
    }

    pub fn platform_defaults() -> Self {
        let fallback = || std::env::temp_dir().join("pixiv-manager");
        Self::from_bases(
            dirs::config_dir().unwrap_or_else(fallback),
            dirs::data_local_dir().unwrap_or_else(fallback),
            dirs::cache_dir().unwrap_or_else(fallback),
        )
    }

    /// Layout rule, split out so every platform's input can be tested from
    /// any host.
    ///
    /// Windows resolves data and cache to the same directory, and macOS
    /// resolves config and data to the same one; where that happens each kind
    /// gets an explicit child directory so they never coincide or nest.
    fn from_bases(config_base: PathBuf, data_base: PathBuf, cache_base: PathBuf) -> Self {
        let config_dir = config_base.join(APP_DIR);
        let data_root = data_base.join(APP_DIR);
        let cache_root = cache_base.join(APP_DIR);

        let shared_root = cache_root.starts_with(&data_root) || data_root.starts_with(&cache_root);
        let data_dir = if shared_root || data_root == config_dir {
            data_root.join("data")
        } else {
            data_root
        };
        let cache_dir = if shared_root {
            cache_root.join("cache")
        } else {
            cache_root
        };
        Self {
            config_dir,
            data_dir,
            cache_dir,
        }
    }

    fn from_override(value: Option<std::ffi::OsString>) -> Option<Self> {
        let value = value?;
        if value.is_empty() {
            return None;
        }
        Some(Self::rooted(PathBuf::from(value)))
    }

    pub fn config_file(&self) -> PathBuf {
        self.config_dir.join("config.toml")
    }

    pub fn library_dir(&self) -> PathBuf {
        self.data_dir.join("library")
    }

    pub fn bookmarks_file(&self) -> PathBuf {
        self.library_dir().join("bookmarks.json")
    }

    pub fn download_queue_file(&self) -> PathBuf {
        self.library_dir().join("download_queue.json")
    }

    pub fn colors_file(&self) -> PathBuf {
        self.library_dir().join("colors.json")
    }

    pub fn meta_dir(&self) -> PathBuf {
        self.library_dir().join("meta")
    }

    pub fn viewer_transforms_file(&self) -> PathBuf {
        self.library_dir().join("viewer_transforms.json")
    }

    pub fn image_cache_dir(&self) -> PathBuf {
        self.cache_dir.join("images")
    }

    pub fn twitter_cookie_file(&self) -> PathBuf {
        self.config_dir.join("twitter_cookies.txt")
    }

    pub fn bilibili_cookie_file(&self) -> PathBuf {
        self.config_dir.join("bilibili_cookies.txt")
    }

    pub fn twitter_graphql_file(&self) -> PathBuf {
        self.config_dir.join("twitter_graphql.json")
    }

    /// Default download location when the user has not chosen one.
    pub fn default_gallery_dir(&self) -> PathBuf {
        dirs::picture_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("pixiv")
    }

    /// Data roots used before the explicit `data/` component was introduced.
    /// Empty on platforms whose layout never changed.
    pub fn legacy_data_roots(&self) -> Vec<PathBuf> {
        if self.data_dir.file_name().and_then(|n| n.to_str()) != Some("data") {
            return Vec::new();
        }
        self.data_dir
            .parent()
            .map(|parent| vec![parent.to_path_buf()])
            .unwrap_or_default()
    }
}

const APP_DIR: &str = "pixiv-manager";

/// Highest-priority path override.
pub const OVERRIDE_ENV: &str = "PIXIV_MANAGER_CONFIG_DIR";

/// Render a path for display in the UI.
pub fn display(path: &Path) -> String {
    path.display().to_string()
}

/// Image cache location written by releases before the cache moved to the OS
/// cache directory. `None` for overridden layouts: the legacy path would point
/// at real user data.
pub fn legacy_image_cache_dir() -> Option<PathBuf> {
    if cfg!(feature = "test-util") || std::env::var_os(OVERRIDE_ENV).is_some_and(|v| !v.is_empty())
    {
        return None;
    }
    dirs::data_dir().map(|d| d.join(APP_DIR).join("cache"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rooted_puts_the_cache_below_the_root() {
        let root = std::env::temp_dir().join("pm-paths-root");
        let paths = AppPaths::rooted(root.clone());
        assert_eq!(paths.config_dir, root);
        assert_eq!(paths.data_dir, root);
        assert_eq!(paths.cache_dir, root.join("cache"));
    }

    #[test]
    fn derived_paths_live_under_their_directory() {
        let root = std::env::temp_dir().join("pm-paths-derived");
        let paths = AppPaths::rooted(root.clone());
        assert_eq!(paths.config_file(), root.join("config.toml"));
        assert_eq!(paths.library_dir(), root.join("library"));
        assert_eq!(paths.colors_file(), root.join("library/colors.json"));
        assert_eq!(paths.meta_dir(), root.join("library/meta"));
        assert_eq!(
            paths.viewer_transforms_file(),
            root.join("library/viewer_transforms.json")
        );
        assert_eq!(
            paths.twitter_cookie_file(),
            root.join("twitter_cookies.txt")
        );
        assert_eq!(
            paths.bilibili_cookie_file(),
            root.join("bilibili_cookies.txt")
        );
        assert_eq!(
            paths.twitter_graphql_file(),
            root.join("twitter_graphql.json")
        );
        assert_eq!(paths.image_cache_dir(), root.join("cache/images"));
    }

    #[test]
    fn default_gallery_dir_is_pictures_pixiv() {
        let paths = AppPaths::rooted(std::env::temp_dir());
        assert_eq!(
            paths
                .default_gallery_dir()
                .file_name()
                .and_then(|n| n.to_str()),
            Some("pixiv")
        );
    }

    #[test]
    fn empty_override_is_ignored() {
        assert!(AppPaths::from_override(Some(std::ffi::OsString::new())).is_none());
        assert!(AppPaths::from_override(None).is_none());
    }

    #[test]
    fn override_redirects_every_directory() {
        let root = std::env::temp_dir().join("pm-override");
        let paths = AppPaths::from_override(Some(root.clone().into_os_string())).unwrap();
        assert_eq!(paths.config_file(), root.join("config.toml"));
        assert_eq!(paths.library_dir(), root.join("library"));
    }

    #[test]
    fn discovered_paths_are_absolute() {
        let paths = AppPaths::discover();
        for dir in [&paths.config_dir, &paths.data_dir, &paths.cache_dir] {
            assert!(dir.is_absolute(), "{dir:?} must be absolute");
        }
    }

    #[test]
    fn windows_layout_separates_data_and_cache() {
        // Windows resolves local data and cache to the same directory.
        let roaming = PathBuf::from(r"C:\Users\u\AppData\Roaming");
        let local = PathBuf::from(r"C:\Users\u\AppData\Local");
        let paths = AppPaths::from_bases(roaming.clone(), local.clone(), local.clone());

        assert_eq!(paths.config_dir, roaming.join(APP_DIR));
        assert_eq!(paths.data_dir, local.join(APP_DIR).join("data"));
        assert_eq!(paths.cache_dir, local.join(APP_DIR).join("cache"));
        assert_separate(&paths);
    }

    #[test]
    fn macos_layout_separates_config_and_data() {
        let support = PathBuf::from("/Users/u/Library/Application Support");
        let paths = AppPaths::from_bases(
            support.clone(),
            support,
            PathBuf::from("/Users/u/Library/Caches"),
        );
        assert_eq!(
            paths.config_dir,
            PathBuf::from("/Users/u/Library/Application Support/pixiv-manager")
        );
        assert_eq!(
            paths.data_dir,
            PathBuf::from("/Users/u/Library/Application Support/pixiv-manager/data")
        );
        assert_eq!(
            paths.cache_dir,
            PathBuf::from("/Users/u/Library/Caches/pixiv-manager")
        );
        assert_separate(&paths);
    }

    #[test]
    fn linux_layout_keeps_the_xdg_directories() {
        let paths = AppPaths::from_bases(
            PathBuf::from("/home/u/.config"),
            PathBuf::from("/home/u/.local/share"),
            PathBuf::from("/home/u/.cache"),
        );
        assert_eq!(
            paths.data_dir,
            PathBuf::from("/home/u/.local/share/pixiv-manager")
        );
        assert_eq!(
            paths.cache_dir,
            PathBuf::from("/home/u/.cache/pixiv-manager")
        );
        assert_separate(&paths);
    }

    fn assert_separate(paths: &AppPaths) {
        assert_ne!(paths.config_dir, paths.data_dir);
        assert_ne!(paths.data_dir, paths.cache_dir);
        assert_ne!(paths.config_dir, paths.cache_dir);
        assert!(!paths.cache_dir.starts_with(&paths.data_dir));
        assert!(!paths.data_dir.starts_with(&paths.cache_dir));
    }

    #[test]
    fn host_os_layout_is_separate() {
        assert_separate(&AppPaths::platform_defaults());
    }

    /// The override deliberately collapses config and data onto one root
    /// (tests, portable installs), but never onto the cache.
    #[test]
    fn override_collapses_config_and_data_only() {
        let paths = AppPaths::rooted(std::env::temp_dir().join("pm-override-collapse"));
        assert_eq!(paths.config_dir, paths.data_dir);
        assert_ne!(paths.cache_dir, paths.data_dir);
    }

    #[test]
    fn legacy_data_root_is_the_parent_of_data() {
        let paths = AppPaths {
            config_dir: PathBuf::from("/cfg"),
            data_dir: PathBuf::from("/app/data"),
            cache_dir: PathBuf::from("/cache"),
        };
        assert_eq!(paths.legacy_data_roots(), vec![PathBuf::from("/app")]);
    }
}

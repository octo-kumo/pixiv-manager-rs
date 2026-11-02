//! The single authority for *where the application stores its files*.
//!
//! Every module that needs a path asks here (directly or through the thin
//! wrappers in [`crate::io::config`], [`crate::io::library`], …). Nothing else
//! in the codebase may call `dirs::*` or assume a Linux `~/.config` layout —
//! that is what made the previous Windows port attempt tangle OS checks
//! through the downloader, the reducer and the settings UI.
//!
//! Layout, per platform (the `dirs` crate supplies the OS conventions):
//!
//! | kind              | Linux                 | Windows          | macOS                            |
//! |-------------------|-----------------------|------------------|----------------------------------|
//! | config            | `$XDG_CONFIG_HOME`    | `%APPDATA%`      | `~/Library/Application Support`  |
//! | data (library)    | `$XDG_DATA_HOME`      | `%LOCALAPPDATA%` | `~/Library/Application Support`  |
//! | cache (images)    | `$XDG_CACHE_HOME`     | `%LOCALAPPDATA%` | `~/Library/Caches`               |
//!
//! All three live under a `pixiv-manager` subdirectory of the OS directory,
//! except the cache, which uses `<cache>/pixiv-manager`.
//!
//! # Overrides
//!
//! `PIXIV_MANAGER_CONFIG_DIR` is the highest-priority override: it collapses
//! config *and* data onto one directory (`$VAR/config.toml`, `$VAR/library`,
//! `$VAR/cache`). It exists for tests and portable/self-contained installs and
//! is the only mechanism the test suite may use to stay off real user data.

use std::path::{Path, PathBuf};

/// Application-owned directories.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppPaths {
    /// Configuration: `config.toml` (holds the OAuth refresh token), plus the
    /// optional Twitter/Bilibili cookie files and the GraphQL override.
    pub config_dir: PathBuf,
    /// Persistent data: the bookmark/queue library, colours, metadata cache
    /// and viewer transforms.
    pub data_dir: PathBuf,
    /// Disposable cache: downloaded image bytes. Losing it is always safe.
    pub cache_dir: PathBuf,
}

impl AppPaths {
    /// Resolve the application paths, honouring `PIXIV_MANAGER_CONFIG_DIR`
    /// first and the OS conventions second.
    ///
    /// Recomputed on every call (it is a handful of environment lookups) so
    /// that a test which redirects `PIXIV_MANAGER_CONFIG_DIR` at runtime —
    /// or a subprocess started with it — always sees the redirect.
    pub fn discover() -> Self {
        if let Some(paths) = Self::from_override(std::env::var_os(OVERRIDE_ENV)) {
            return paths;
        }
        // Test builds (feature `test-util`, forced on by the package's own
        // dev-dependency) must never read or write the real user config:
        // several tests persist a fake token via `save_config`, which used to
        // clobber a developer's saved login. Give them a per-process temp
        // directory unless they asked for a specific one.
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

    /// Every application directory under one root. Used for `PIXIV_MANAGER_
    /// CONFIG_DIR` overrides (tests, portable installs) where config, data and
    /// cache should stay together and side-effect free.
    pub fn rooted(root: PathBuf) -> Self {
        Self {
            cache_dir: root.join("cache"),
            config_dir: root.clone(),
            data_dir: root,
        }
    }

    /// The OS-native default locations.
    ///
    /// `dirs` returns `None` in exotic environments (no `$HOME`, a container
    /// with a stripped environment); falling back to a temp directory keeps
    /// the app usable there instead of panicking.
    pub fn platform_defaults() -> Self {
        let fallback = || std::env::temp_dir().join("pixiv-manager");
        let config_dir = dirs::config_dir()
            .map(|d| d.join(APP_DIR))
            .unwrap_or_else(fallback);
        let data_dir = dirs::data_local_dir()
            .map(|d| d.join(APP_DIR))
            .unwrap_or_else(fallback);
        let cache_dir = dirs::cache_dir()
            .map(|d| d.join(APP_DIR))
            .unwrap_or_else(fallback);
        Self {
            config_dir,
            data_dir,
            cache_dir,
        }
    }

    /// `$PIXIV_MANAGER_CONFIG_DIR`, when set to a non-empty value.
    fn from_override(value: Option<std::ffi::OsString>) -> Option<Self> {
        let value = value?;
        if value.is_empty() {
            return None;
        }
        Some(Self::rooted(PathBuf::from(value)))
    }

    /// Where downloads go when the user has not chosen a gallery directory:
    /// `Pictures/pixiv` on every platform (`X:\Users\me\Pictures\pixiv`,
    /// `~/Pictures/pixiv`, …), falling back to `./pixiv` when the OS has no
    /// Pictures directory.
    ///
    /// This is a *user media* location rather than an application directory,
    /// but it is still a path decision, so it lives with the others.
    pub fn default_gallery_dir(&self) -> PathBuf {
        dirs::picture_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("pixiv")
    }

    // ── Files derived from the directories ──────────────────────────

    /// `config.toml` — the settings (and the OAuth refresh token).
    pub fn config_file(&self) -> PathBuf {
        self.config_dir.join("config.toml")
    }

    /// The bookmark library, download queue, colours, metadata cache and
    /// viewer transforms all live under here (unchanged since the first
    /// release, so existing libraries keep working).
    pub fn library_dir(&self) -> PathBuf {
        self.data_dir.join("library")
    }

    /// The bookmark library JSON.
    pub fn bookmarks_file(&self) -> PathBuf {
        self.library_dir().join("bookmarks.json")
    }

    /// The persisted download queue.
    pub fn download_queue_file(&self) -> PathBuf {
        self.library_dir().join("download_queue.json")
    }

    /// The persisted MMCQ colour palettes.
    pub fn colors_file(&self) -> PathBuf {
        self.library_dir().join("colors.json")
    }

    /// Per-illust metadata (detail + comments), one file per id.
    pub fn meta_dir(&self) -> PathBuf {
        self.library_dir().join("meta")
    }

    /// The viewer's per-illust rotation/flip state.
    pub fn viewer_transforms_file(&self) -> PathBuf {
        self.library_dir().join("viewer_transforms.json")
    }

    /// Where the image byte cache keeps its files.
    pub fn image_cache_dir(&self) -> PathBuf {
        self.cache_dir.join("images")
    }

    /// Conventional location of the browser-exported X/Twitter cookies.
    pub fn twitter_cookie_file(&self) -> PathBuf {
        self.config_dir.join("twitter_cookies.txt")
    }

    /// Conventional location of the browser-exported Bilibili cookies.
    pub fn bilibili_cookie_file(&self) -> PathBuf {
        self.config_dir.join("bilibili_cookies.txt")
    }

    /// The runtime Twitter GraphQL query-id override written by Settings.
    pub fn twitter_graphql_file(&self) -> PathBuf {
        self.config_dir.join("twitter_graphql.json")
    }
}

/// A user-facing rendering of `path` for hints in the settings UI, so no
/// screen ever hard-codes a Linux-specific location.
pub fn display(path: &Path) -> String {
    path.display().to_string()
}

/// The directory name under the OS config/data/cache directories.
const APP_DIR: &str = "pixiv-manager";

/// Highest-priority override, shared by the GUI, the CLI and the tests.
pub const OVERRIDE_ENV: &str = "PIXIV_MANAGER_CONFIG_DIR";

/// The pre-`AppPaths` cache location (`<data>/pixiv-manager/cache`), which
/// older releases used for image bytes.
///
/// Returned only for OS-default installations: when the user (or a test)
/// redirected `PIXIV_MANAGER_CONFIG_DIR`, the legacy path would point at real
/// user data and must not be touched.
pub fn legacy_image_cache_dir() -> Option<PathBuf> {
    // Test builds must never touch the real data directory: migrating would
    // *move* a developer's real cache into the test's temp root.
    if cfg!(feature = "test-util") || override_env_is_set() {
        return None;
    }
    dirs::data_dir().map(|d| d.join(APP_DIR).join("cache"))
}

fn override_env_is_set() -> bool {
    std::env::var_os(OVERRIDE_ENV).is_some_and(|v| !v.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn rooted_collapses_config_and_data_onto_one_root() {
        let root = PathBuf::from("C:\\tmp\\pm").join("x");
        let paths = AppPaths::rooted(root.clone());
        assert_eq!(paths.config_dir, root);
        assert_eq!(paths.data_dir, root);
        assert_eq!(paths.cache_dir, root.join("cache"));
    }

    #[test]
    fn derived_files_have_the_documented_shape() {
        let root = std::env::temp_dir().join(format!("pm-paths-{}", std::process::id()));
        let paths = AppPaths::rooted(root.clone());
        assert_eq!(paths.config_file(), root.join("config.toml"));
        assert_eq!(paths.library_dir(), root.join("library"));
        assert_eq!(
            paths.colors_file(),
            root.join("library").join("colors.json")
        );
        assert_eq!(paths.meta_dir(), root.join("library").join("meta"));
        assert_eq!(
            paths.viewer_transforms_file(),
            root.join("library").join("viewer_transforms.json")
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
        assert_eq!(paths.image_cache_dir(), root.join("cache").join("images"));
    }

    #[test]
    fn default_gallery_dir_is_pictures_pixiv() {
        let paths = AppPaths::rooted(std::env::temp_dir());
        let gallery = paths.default_gallery_dir();
        assert_eq!(gallery.file_name().and_then(|n| n.to_str()), Some("pixiv"));
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

    /// Discovered paths must be usable no matter how the app was started: no
    /// relative paths (a `cd` would silently move the library) and no empty
    /// components.
    #[test]
    fn discovered_paths_are_absolute_and_non_empty() {
        let paths = AppPaths::discover();
        for dir in [&paths.config_dir, &paths.data_dir, &paths.cache_dir] {
            assert!(dir.is_absolute(), "{dir:?} must be absolute");
            assert!(dir.file_name().is_some(), "{dir:?} must name a directory");
        }
    }

    /// The per-platform directory kinds must not collapse onto each other in a
    /// way that puts disposable cache data where backups look for user data.
    #[test]
    fn cache_is_separate_from_config_and_library() {
        let paths = AppPaths::discover();
        assert_ne!(paths.cache_dir, paths.config_dir);
        assert_ne!(paths.cache_dir, paths.library_dir());
    }
}

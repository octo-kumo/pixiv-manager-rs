//! Shared persistence primitives.
//!
//! Two rules, enforced by review:
//!
//! 1. **Every persistent application write goes through [`write_atomic`] or
//!    [`save_json_atomic`].** No module may keep its own temp-file-plus-rename
//!    routine: `colors.json`, `meta/*.json`, `viewer_transforms.json`,
//!    `config.toml`, the bookmark library, the download queue and the Twitter
//!    GraphQL override all share this one implementation. (Previously five
//!    slightly different copies existed, three of which used the Windows-
//!    hostile `remove_file(dest); rename(tmp, dest)` sequence.)
//! 2. **Crash safety means "old or new, never half".** The bytes are written
//!    and `fsync`ed to a temp file in the destination directory, then swapped
//!    in with [`crate::platform::replace_file`], which is a single atomic
//!    operation on both Unix and Windows. A crash may leave a stale `.tmp`
//!    file behind — readers ignore those, and the next write overwrites it.

pub mod colors;
pub mod config;
pub mod download;
pub mod library;
pub mod meta_cache;
pub mod paths;

use std::io::Write;
use std::path::{Path, PathBuf};

use crate::platform::replace_file;

/// Read a file to a string with optional profiling instrumentation.
pub fn read_file(path: &Path) -> Option<String> {
    #[cfg(feature = "profiling")]
    let start = std::time::Instant::now();
    let content = std::fs::read_to_string(path).ok()?;
    #[cfg(feature = "profiling")]
    crate::profiling::tracing::record_file(
        crate::profiling::tracing::FileOp::Read,
        path.display().to_string(),
        content.len(),
        start.elapsed(),
    );
    Some(content)
}

/// Serialize `value` as pretty JSON and write it atomically.
pub fn save_json_atomic<T: serde::Serialize>(value: &T, path: &Path) -> std::io::Result<()> {
    let content = serde_json::to_string_pretty(value)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    write_atomic(path, content.as_bytes())
}

/// Serialize `value` as compact JSON and write it atomically.
///
/// Used for the per-illust metadata cache, where the files are numerous and
/// nobody reads them by hand.
pub fn save_json_compact_atomic<T: serde::Serialize>(
    value: &T,
    path: &Path,
) -> std::io::Result<()> {
    let content = serde_json::to_string(value)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    write_atomic(path, content.as_bytes())
}

/// Write `bytes` to `path`, replacing any previous contents atomically.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    write_atomic_inner(path, bytes, Visibility::Default)
}

/// Write `bytes` to `path` atomically, owner-only (`0600`) on Unix.
///
/// Used for `config.toml`, which holds the Pixiv OAuth refresh token
/// (equivalent to the account password). On Windows the file inherits the
/// user profile's ACLs — the same protection every other per-user
/// configuration file has — which is why Credential Manager integration stays
/// a separate, future milestone.
pub fn write_atomic_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    write_atomic_inner(path, bytes, Visibility::OwnerOnly)
}

#[derive(Clone, Copy)]
enum Visibility {
    Default,
    OwnerOnly,
}

/// The temp file used for `path`: same directory (so the final swap is a
/// same-filesystem rename) and process-unique (so a CLI and the GUI writing
/// the same config simultaneously cannot corrupt each other's temp file).
fn temp_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".tmp{}", std::process::id()));
    path.with_file_name(name)
}

fn write_atomic_inner(path: &Path, bytes: &[u8], visibility: Visibility) -> std::io::Result<()> {
    // Unix applies the mode while creating the temp file; Windows relies on
    // the user-profile ACLs, so the flag is intentionally unused there.
    #[cfg(not(unix))]
    let _ = visibility;
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let tmp = temp_path(path);

    #[cfg(feature = "profiling")]
    let start = std::time::Instant::now();

    let write_result = (|| -> std::io::Result<()> {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        // Create the temp file with the final permissions so the refresh token
        // is never briefly world-readable (a `create` + later `chmod` would
        // leave exactly that window).
        #[cfg(unix)]
        if matches!(visibility, Visibility::OwnerOnly) {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&tmp)?;
        #[cfg(unix)]
        if matches!(visibility, Visibility::OwnerOnly) {
            use std::os::unix::fs::PermissionsExt;
            // A leftover temp file from a crashed write may carry the old
            // (looser) mode; `mode` only applies when the file is created.
            file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }
        file.write_all(bytes)?;
        // Durability: the contents must be on disk before the rename makes
        // them visible under the final name.
        file.sync_all()?;
        drop(file);
        replace_file(&tmp, path)
    })();

    if let Err(e) = write_result {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }

    #[cfg(feature = "profiling")]
    crate::profiling::tracing::record_file(
        crate::profiling::tracing::FileOp::Write,
        path.display().to_string(),
        bytes.len(),
        start.elapsed(),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Serialize;

    #[derive(Serialize, serde::Deserialize)]
    struct Sample {
        value: u32,
    }

    #[test]
    fn writes_and_overwrites_repeatedly() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        for i in 0..5u32 {
            write_atomic(&path, format!("v{i}").as_bytes()).unwrap();
            assert_eq!(std::fs::read_to_string(&path).unwrap(), format!("v{i}"));
        }
        // No temp litter survives a successful write.
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "temp files left: {leftovers:?}");
    }

    #[test]
    fn creates_missing_parent_directories() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a").join("b").join("c.json");
        save_json_atomic(&Sample { value: 1 }, &path).unwrap();
        let back: Sample = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(back.value, 1);
    }

    #[test]
    fn compact_json_is_a_single_line() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("meta.json");
        save_json_compact_atomic(&Sample { value: 2 }, &path).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(!content.contains('\n'), "compact JSON must be one line");
    }

    #[cfg(unix)]
    #[test]
    fn private_writes_are_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        write_atomic_private(&path, b"token = \"x\"").unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o077, 0, "config must not be group/other readable");
    }

    /// The refresh token must never exist greedily-readable, not even for the
    /// instant between creating the temp file and renaming it.
    #[cfg(unix)]
    #[test]
    fn private_writes_fix_a_stale_world_readable_temp_file() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let tmp = temp_path(&path);
        std::fs::write(&tmp, b"stale").unwrap();
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o644)).unwrap();
        write_atomic_private(&path, b"fresh").unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o077, 0, "stale temp file mode leaked through");
    }

    #[test]
    fn read_file_reads_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.txt");
        write_atomic(&path, b"hello").unwrap();
        assert_eq!(read_file(&path).as_deref(), Some("hello"));
        assert_eq!(read_file(&dir.path().join("missing.txt")), None);
    }
}

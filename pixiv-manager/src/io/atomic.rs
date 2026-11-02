//! The one durable-write path for application state.
//!
//! A write creates an exclusively-created temp file next to the destination,
//! fills and fsyncs it, then hands it to [`crate::platform::fs::replace_file`].
//! The temp file is deleted only when the replacement proves the destination
//! is intact or was updated; after an ambiguous failure it is kept as the
//! recovery copy.

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use crate::platform::fs::{move_file, replace_file, ReplaceOutcome};

/// Marks a file as one of our temp files. Cleanup and cache code must use
/// [`is_atomic_temp_file`] rather than re-deriving this pattern.
pub const TEMP_MARKER: &str = ".pixiv-manager-tmp-";

const CREATE_ATTEMPTS: u32 = 32;

static REPLACE_LOCK: Mutex<()> = Mutex::new(());

/// Why a durable write did not happen, and what is on disk afterwards.
#[derive(Debug)]
pub enum AtomicWriteError {
    Serialize(serde_json::Error),
    /// The destination still holds its previous content and no temp file was
    /// left behind.
    Intact {
        source: io::Error,
    },
    /// The replacement could not be confirmed. `recovery` points at the temp
    /// file holding the complete new content, when it still exists.
    Ambiguous {
        source: io::Error,
        recovery: Option<PathBuf>,
    },
}

impl std::fmt::Display for AtomicWriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Serialize(e) => write!(f, "could not serialize: {e}"),
            Self::Intact { source } => {
                write!(f, "write failed, previous file unchanged: {source}")
            }
            Self::Ambiguous { source, recovery } => {
                write!(
                    f,
                    "write failed with an ambiguous filesystem state: {source}"
                )?;
                match recovery {
                    Some(path) => write!(f, "; new content kept at {}", path.display()),
                    None => write!(f, "; no recovery file remains"),
                }
            }
        }
    }
}

impl std::error::Error for AtomicWriteError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Serialize(e) => Some(e),
            Self::Intact { source } | Self::Ambiguous { source, .. } => Some(source),
        }
    }
}

#[derive(Clone, Copy)]
enum Visibility {
    Default,
    OwnerOnly,
}

pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), AtomicWriteError> {
    write_inner(path, bytes, Visibility::Default)
}

/// Owner-only (`0600`) on Unix. Used for files holding credentials.
pub fn write_atomic_private(path: &Path, bytes: &[u8]) -> Result<(), AtomicWriteError> {
    write_inner(path, bytes, Visibility::OwnerOnly)
}

pub fn save_json_atomic<T: serde::Serialize>(
    value: &T,
    path: &Path,
) -> Result<(), AtomicWriteError> {
    let content = serde_json::to_string_pretty(value).map_err(AtomicWriteError::Serialize)?;
    write_atomic(path, content.as_bytes())
}

pub fn save_json_compact_atomic<T: serde::Serialize>(
    value: &T,
    path: &Path,
) -> Result<(), AtomicWriteError> {
    let content = serde_json::to_string(value).map_err(AtomicWriteError::Serialize)?;
    write_atomic(path, content.as_bytes())
}

/// Whether `name` was produced by [`write_atomic`].
pub fn is_atomic_temp_file(name: &std::ffi::OsStr) -> bool {
    name.to_string_lossy().contains(TEMP_MARKER)
}

/// Delete temp files older than `max_age` in `dir` (not recursive).
///
/// The age check keeps a live writer in another process from losing its temp
/// file; only recognizable leftovers from a crash are removed.
pub fn cleanup_stale_atomic_temps(dir: &Path, max_age: Duration) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut removed = 0;
    for entry in entries.flatten() {
        if !is_atomic_temp_file(&entry.file_name()) {
            continue;
        }
        let stale = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .is_some_and(|age| age >= max_age);
        if stale && std::fs::remove_file(entry.path()).is_ok() {
            removed += 1;
        }
    }
    removed
}

fn write_inner(path: &Path, bytes: &[u8], visibility: Visibility) -> Result<(), AtomicWriteError> {
    let intact = |source| AtomicWriteError::Intact { source };
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir).map_err(intact)?;

    let (mut file, tmp) = create_temp(path, dir, visibility).map_err(intact)?;
    let written = (|| -> io::Result<()> {
        file.write_all(bytes)?;
        file.sync_all()
    })();
    drop(file);
    if let Err(source) = written {
        let _ = std::fs::remove_file(&tmp);
        return Err(intact(source));
    }

    // Two replacements of the same destination at once make Windows fail with
    // ERROR_UNABLE_TO_REMOVE_REPLACED, so this step is serialised process-wide.
    let _replace = REPLACE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    match replace_file(&tmp, path) {
        ReplaceOutcome::Replaced => Ok(()),
        ReplaceOutcome::SourceIntact(source) => {
            let _ = std::fs::remove_file(&tmp);
            Err(AtomicWriteError::Intact { source })
        }
        ReplaceOutcome::Indeterminate(source) => {
            // The destination may have been destroyed by the failed replace;
            // try to establish it from the temp before giving up.
            if !path.exists() && move_file(&tmp, path).is_ok() {
                return Ok(());
            }
            let recovery = tmp.exists().then(|| tmp.clone());
            Err(AtomicWriteError::Ambiguous { source, recovery })
        }
    }
}

fn create_temp(
    dest: &Path,
    dir: &Path,
    visibility: Visibility,
) -> io::Result<(std::fs::File, PathBuf)> {
    let mut last = io::Error::new(io::ErrorKind::AlreadyExists, "no unique temp name");
    for _ in 0..CREATE_ATTEMPTS {
        let path = dir.join(temp_name(dest));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        if matches!(visibility, Visibility::OwnerOnly) {
            use std::os::unix::fs::OpenOptionsExt;
            // Set at creation so the file is never briefly world-readable.
            options.mode(0o600);
        }
        match options.open(&path) {
            Ok(file) => return Ok((file, path)),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => last = e,
            Err(e) => return Err(e),
        }
    }
    Err(last)
}

/// `config.toml.pixiv-manager-tmp-<pid>-<sequence>-<nonce>`.
///
/// The process nonce makes a name from a previous process with a recycled PID
/// effectively impossible to collide with; `create_new` still arbitrates.
fn temp_name(dest: &Path) -> std::ffi::OsString {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let mut name = dest
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_else(|| std::ffi::OsString::from("state"));
    name.push(format!(
        "{}{}-{}-{:016x}",
        TEMP_MARKER,
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed),
        process_nonce()
    ));
    name
}

fn process_nonce() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    use std::sync::OnceLock;
    static NONCE: OnceLock<u64> = OnceLock::new();
    *NONCE.get_or_init(|| {
        let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
        hasher.write_u32(std::process::id());
        hasher.write_u128(
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default(),
        );
        hasher.finish()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn writes_and_overwrites_repeatedly() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        for i in 0..5u32 {
            write_atomic(&path, format!("v{i}").as_bytes()).unwrap();
            assert_eq!(std::fs::read_to_string(&path).unwrap(), format!("v{i}"));
        }
        assert_eq!(names(dir.path()), vec!["state.json".to_string()]);
    }

    #[test]
    fn concurrent_writes_use_distinct_temp_files() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("config.toml");
        let created = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));

        std::thread::scope(|scope| {
            for _ in 0..16 {
                let created = created.clone();
                let dest = dest.clone();
                let dir = dir.path().to_path_buf();
                scope.spawn(move || {
                    let (file, tmp) = create_temp(&dest, &dir, Visibility::Default).unwrap();
                    created.lock().unwrap().push(tmp.clone());
                    drop(file);
                    std::fs::write(&tmp, b"x").unwrap();
                });
            }
        });

        let paths = created.lock().unwrap().clone();
        let unique: std::collections::HashSet<_> = paths.iter().collect();
        assert_eq!(unique.len(), paths.len(), "temp files collided");
        assert!(
            paths
                .iter()
                .all(|p| is_atomic_temp_file(p.file_name().unwrap())),
            "temp names must be recognizable: {paths:?}"
        );
    }

    /// A pre-existing file that looks like one of our temp files must not be
    /// truncated or reused by a later write.
    #[test]
    fn stale_temp_files_are_not_reused() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("config.toml");
        let stale = dir.path().join("config.toml.pixiv-manager-tmp-1-0-0");
        std::fs::write(&stale, b"stale").unwrap();

        write_atomic(&dest, b"fresh").unwrap();

        assert_eq!(std::fs::read(&stale).unwrap(), b"stale");
        assert_eq!(std::fs::read(&dest).unwrap(), b"fresh");
        cleanup_stale_atomic_temps(dir.path(), Duration::ZERO);
        assert!(!stale.exists(), "stale temp must be collected");
        assert!(dest.exists(), "cleanup must not touch the destination");
    }

    #[test]
    fn cleanup_ignores_unrelated_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("notes.txt"), b"keep").unwrap();
        std::fs::write(dir.path().join("config.toml"), b"keep").unwrap();
        std::fs::write(dir.path().join("x.tmp"), b"keep").unwrap();
        assert_eq!(cleanup_stale_atomic_temps(dir.path(), Duration::ZERO), 0);
        assert_eq!(names(dir.path()).len(), 3);
    }

    #[cfg(unix)]
    #[test]
    fn failed_temp_creation_leaves_destination_untouched() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("locked");
        std::fs::create_dir(&sub).unwrap();
        let dest = sub.join("state.json");
        std::fs::write(&dest, b"old").unwrap();
        std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o500)).unwrap();

        let result = write_atomic(&dest, b"new");
        std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o700)).unwrap();

        assert!(
            matches!(result, Err(AtomicWriteError::Intact { .. })),
            "expected an intact failure, got {result:?}"
        );
        assert_eq!(std::fs::read(&dest).unwrap(), b"old");
    }

    #[test]
    fn creates_missing_parent_directories() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a").join("b").join("c.json");
        save_json_atomic(&serde_json::json!({"v": 1}), &path).unwrap();
        assert!(path.is_file());
    }

    #[test]
    fn compact_json_is_a_single_line() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("meta.json");
        save_json_compact_atomic(&serde_json::json!({"v": 2}), &path).unwrap();
        assert!(!std::fs::read_to_string(&path).unwrap().contains('\n'));
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

    /// A leftover temp file carrying the old 0644 mode must not leak through.
    #[cfg(unix)]
    #[test]
    fn private_writes_ignore_stale_temp_mode() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let stale = dir.path().join("config.toml.pixiv-manager-tmp-1-0-0");
        std::fs::write(&stale, b"stale").unwrap();
        std::fs::set_permissions(&stale, std::fs::Permissions::from_mode(0o644)).unwrap();

        write_atomic_private(&path, b"fresh").unwrap();

        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o077, 0, "stale temp permissions leaked through");
    }
}

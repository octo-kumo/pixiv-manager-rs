//! Atomic file replacement — the one primitive every persistent writer uses.
//!
//! Callers write a temp file next to the destination and then hand both paths
//! to [`replace_file`], which swaps them in a single filesystem operation:
//!
//! * **Unix** — `rename(2)`: atomic, and the destination is replaced without
//!   ever disappearing. The parent directory is then fsynced so the rename
//!   itself survives a power loss.
//! * **Windows** — `ReplaceFileW` for an existing destination (the documented
//!   atomic replace API; it also keeps the original ACL and attributes), and
//!   `MoveFileExW(MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH)` when
//!   there is nothing to replace yet. Before this, the code did
//!   `remove_file(dest)` + `rename`, which leaves a window where the config /
//!   library simply does not exist.
//!
//! Both paths retry on the transient sharing violations that Windows
//! antivirus/indexer handles cause when they hold the destination open.

use std::io;
use std::path::Path;

/// How many times a Windows replace is retried before giving up.
#[cfg(windows)]
const REPLACE_RETRIES: u32 = 12;

/// Replace `src` with `dest` (both must be on the same filesystem).
///
/// On success `src` no longer exists and `dest` holds its contents. On failure
/// the previous `dest` (if any) is left untouched and `src` may still exist —
/// callers treat a failure as "the write did not happen".
pub fn replace_file(src: &Path, dest: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        unix::rename_over(src, dest)
    }
    #[cfg(windows)]
    {
        windows::replace(src, dest)
    }
    #[cfg(not(any(unix, windows)))]
    {
        // No platform-specific guarantee available: fall back to the most
        // portable rename. (Not reachable on the targets we ship.)
        std::fs::rename(src, dest)
    }
}

#[cfg(unix)]
mod unix {
    use std::io;
    use std::path::Path;

    pub(super) fn rename_over(src: &Path, dest: &Path) -> io::Result<()> {
        std::fs::rename(src, dest)?;
        // Make the directory entry itself durable. Best effort: some
        // filesystems (and every non-Unix target) cannot sync a directory,
        // and failing here would report an error for a write that succeeded.
        if let Some(parent) = dest.parent() {
            if let Ok(dir) = std::fs::File::open(parent) {
                let _ = dir.sync_all();
            }
        }
        Ok(())
    }
}

#[cfg(windows)]
mod windows {
    use std::io;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;
    use std::time::Duration;

    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, ReplaceFileW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
        REPLACEFILE_IGNORE_MERGE_ERRORS,
    };

    const ERROR_FILE_NOT_FOUND: i32 = 2;
    const ERROR_PATH_NOT_FOUND: i32 = 3;
    const ERROR_ACCESS_DENIED: i32 = 5;
    const ERROR_SHARING_VIOLATION: i32 = 32;
    const ERROR_LOCK_VIOLATION: i32 = 33;

    fn wide(path: &Path) -> Vec<u16> {
        path.as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    fn move_over(src: &Path, dest: &Path) -> io::Result<()> {
        let src_w = wide(src);
        let dest_w = wide(dest);
        // MOVEFILE_WRITE_THROUGH makes the rename durable before returning;
        // REPLACE_EXISTING is what makes this an *overwrite* (and what the old
        // remove-then-rename workaround was emulating).
        let ok = unsafe {
            MoveFileExW(
                src_w.as_ptr(),
                dest_w.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        };
        if ok == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    fn replace_once(src: &Path, dest: &Path) -> io::Result<()> {
        if !dest.exists() {
            return move_over(src, dest);
        }
        let dest_w = wide(dest);
        let src_w = wide(src);
        let ok = unsafe {
            ReplaceFileW(
                dest_w.as_ptr(),
                src_w.as_ptr(),
                std::ptr::null(),
                REPLACEFILE_IGNORE_MERGE_ERRORS,
                std::ptr::null(),
                std::ptr::null(),
            )
        };
        if ok != 0 {
            return Ok(());
        }
        let err = io::Error::last_os_error();
        match err.raw_os_error() {
            // Lost a race with something that removed the destination.
            Some(ERROR_FILE_NOT_FOUND) | Some(ERROR_PATH_NOT_FOUND) => move_over(src, dest),
            _ => Err(err),
        }
    }

    pub(super) fn replace(src: &Path, dest: &Path) -> io::Result<()> {
        let mut last = io::Error::other("replace_file: retries exhausted");
        for attempt in 0..super::REPLACE_RETRIES {
            match replace_once(src, dest) {
                Ok(()) => return Ok(()),
                Err(e) => {
                    // A scanner / indexer / backup agent holding the file open
                    // for a moment is normal on Windows; retry briefly instead
                    // of failing the user's save.
                    let transient = matches!(
                        e.raw_os_error(),
                        Some(ERROR_ACCESS_DENIED)
                            | Some(ERROR_SHARING_VIOLATION)
                            | Some(ERROR_LOCK_VIOLATION)
                    );
                    if !transient {
                        return Err(e);
                    }
                    last = e;
                    std::thread::sleep(Duration::from_millis(5 * u64::from(attempt + 1)));
                }
            }
        }
        Err(last)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaces_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("state.json");
        std::fs::write(&dest, b"old").unwrap();
        let tmp = dir.path().join("state.json.tmp");
        std::fs::write(&tmp, b"new").unwrap();
        replace_file(&tmp, &dest).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"new");
        assert!(!tmp.exists(), "temp file must be consumed");
    }

    #[test]
    fn creates_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("fresh.toml");
        let tmp = dir.path().join("fresh.toml.tmp");
        std::fs::write(&tmp, b"hello").unwrap();
        replace_file(&tmp, &dest).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"hello");
    }

    /// Windows used to be unable to overwrite an existing file at all via the
    /// old `rename`-after-`remove_file` dance; repeated saves are the exact
    /// sequence that must keep working (viewer transforms, colors, bookmarks).
    #[test]
    fn replaces_repeatedly() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("colors.json");
        for i in 0..8u8 {
            let tmp = dir.path().join("colors.json.tmp");
            std::fs::write(&tmp, [i; 3]).unwrap();
            replace_file(&tmp, &dest).unwrap();
            assert_eq!(std::fs::read(&dest).unwrap(), [i; 3]);
        }
    }

    #[test]
    fn missing_source_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("dest.bin");
        std::fs::write(&dest, b"keep me").unwrap();
        let err = replace_file(&dir.path().join("nope.tmp"), &dest).unwrap_err();
        assert!(
            !err.to_string().is_empty(),
            "the failure must describe itself: {err:?}"
        );
        // The destination must be left untouched when the replace fails.
        assert_eq!(std::fs::read(&dest).unwrap(), b"keep me");
    }
}

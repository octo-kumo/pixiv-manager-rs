//! Atomic file replacement.
//!
//! The caller writes a temp file next to the destination and passes both paths
//! to [`replace_file`], which reports one of three states:
//!
//! * [`ReplaceOutcome::Replaced`] — the destination holds the new content;
//! * [`ReplaceOutcome::SourceIntact`] — nothing changed, so the caller may
//!   delete its temp file;
//! * [`ReplaceOutcome::Indeterminate`] — the filesystem may have changed, so
//!   the temp file must be kept as the recovery copy.
//!
//! The third state exists because `ReplaceFileW` is not transactional in every
//! failure mode: it can remove the original before discovering it cannot move
//! the replacement into place. Treating that as "the write failed, delete the
//! temp" would destroy the only complete copy.

use std::io;
use std::path::Path;

/// The result of an attempted replacement.
#[derive(Debug)]
pub enum ReplaceOutcome {
    /// `dest` now holds the new content and `src` is gone.
    Replaced,
    /// Nothing changed: `dest` still holds its previous content and `src` is
    /// still intact.
    SourceIntact(io::Error),
    /// The failure happened after filesystem state changed. `dest` may hold the
    /// previous content or be missing, and `src` may or may not exist. Never
    /// delete `src` in this state.
    Indeterminate(io::Error),
}

/// Replace `dest` with the contents of `src` (same filesystem required).
pub fn replace_file(src: &Path, dest: &Path) -> ReplaceOutcome {
    #[cfg(unix)]
    {
        unix::replace(src, dest)
    }
    #[cfg(windows)]
    {
        windows::replace(src, dest)
    }
    #[cfg(not(any(unix, windows)))]
    {
        match std::fs::rename(src, dest) {
            Ok(()) => ReplaceOutcome::Replaced,
            Err(e) => ReplaceOutcome::Indeterminate(e),
        }
    }
}

/// Move `src` onto `dest`, replacing it, without preserving metadata.
///
/// Used when the destination is missing and for recovery after an
/// [`ReplaceOutcome::Indeterminate`] result.
pub fn move_file(src: &Path, dest: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        unix::move_file(src, dest)
    }
    #[cfg(windows)]
    {
        windows::move_file(src, dest)
    }
    #[cfg(not(any(unix, windows)))]
    {
        std::fs::rename(src, dest)
    }
}

#[cfg(unix)]
mod unix {
    use std::io;
    use std::path::Path;

    use super::ReplaceOutcome;

    fn sync_parent(path: &Path) {
        if let Some(parent) = path.parent() {
            if let Ok(dir) = std::fs::File::open(parent) {
                let _ = dir.sync_all();
            }
        }
    }

    pub(super) fn replace(src: &Path, dest: &Path) -> ReplaceOutcome {
        // rename(2) either replaces the destination or changes nothing.
        match std::fs::rename(src, dest) {
            Ok(()) => {
                sync_parent(dest);
                ReplaceOutcome::Replaced
            }
            Err(e) => ReplaceOutcome::SourceIntact(e),
        }
    }

    pub(super) fn move_file(src: &Path, dest: &Path) -> io::Result<()> {
        std::fs::rename(src, dest)?;
        sync_parent(dest);
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
        REPLACEFILE_IGNORE_ACL_ERRORS, REPLACEFILE_IGNORE_MERGE_ERRORS,
    };

    use super::ReplaceOutcome;

    /// Antivirus scanners, the search indexer and a concurrent replace from
    /// another process hold the destination open for a moment; retried.
    const TRANSIENT_RETRIES: u32 = 16;

    const ERROR_FILE_NOT_FOUND: i32 = 2;
    const ERROR_PATH_NOT_FOUND: i32 = 3;
    const ERROR_ACCESS_DENIED: i32 = 5;
    const ERROR_SHARING_VIOLATION: i32 = 32;
    const ERROR_LOCK_VIOLATION: i32 = 33;
    /// "Unable to remove the file to be replaced": the original is still in
    /// place, so this is a retryable no-op (it happens when another process
    /// replaces the same destination concurrently).
    const ERROR_UNABLE_TO_REMOVE_REPLACED: i32 = 1175;
    /// Documented as "the file being replaced retains its original name"; a
    /// failed ACL/attribute merge reports the same code.
    const ERROR_UNABLE_TO_MOVE_REPLACEMENT: i32 = 1176;

    /// Win32's file APIs, unlike `std::fs`, do not add the `\\?\` verbatim
    /// prefix, so paths at or beyond `MAX_PATH` fail without it. Verbatim paths
    /// also bypass normalization, so `.`/`..` are resolved and `/` converted
    /// before the prefix goes on.
    pub(super) fn wide(path: &Path) -> Vec<u16> {
        const MAX_PATH_LIMIT: usize = 260;
        let mut units: Vec<u16> = path.as_os_str().encode_wide().collect();
        if units.len() >= MAX_PATH_LIMIT - 1 {
            units = verbatim(path);
        }
        units.push(0);
        units
    }

    fn verbatim(path: &Path) -> Vec<u16> {
        let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
        let slash = u16::from(b'\\');
        let units: Vec<u16> = lexical_normalize(&absolute)
            .as_os_str()
            .encode_wide()
            .map(|c| if c == u16::from(b'/') { slash } else { c })
            .collect();

        let unc = units.len() >= 2 && units[0] == slash && units[1] == slash;
        let mut prefixed: Vec<u16> = if unc {
            r"\\?\UNC\".encode_utf16().collect()
        } else {
            r"\\?\".encode_utf16().collect()
        };
        if units.starts_with(&prefixed) {
            return units;
        }
        prefixed.extend_from_slice(if unc { &units[2..] } else { &units[..] });
        prefixed
    }

    fn lexical_normalize(path: &Path) -> std::path::PathBuf {
        use std::path::Component;
        let mut out = std::path::PathBuf::new();
        for component in path.components() {
            match component {
                Component::CurDir => {}
                Component::ParentDir => {
                    if matches!(
                        out.components().next_back(),
                        Some(std::path::Component::Normal(_))
                    ) {
                        out.pop();
                    }
                }
                other => out.push(other.as_os_str()),
            }
        }
        out
    }

    fn is_transient(code: i32) -> bool {
        matches!(
            code,
            ERROR_ACCESS_DENIED
                | ERROR_SHARING_VIOLATION
                | ERROR_LOCK_VIOLATION
                | ERROR_UNABLE_TO_REMOVE_REPLACED
        )
    }

    pub(super) fn move_file(src: &Path, dest: &Path) -> io::Result<()> {
        let src_w = wide(src);
        let dest_w = wide(dest);
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

    fn replace_file_w(dest: &Path, src: &Path, flags: u32) -> io::Result<()> {
        let dest_w = wide(dest);
        let src_w = wide(src);
        let ok = unsafe {
            ReplaceFileW(
                dest_w.as_ptr(),
                src_w.as_ptr(),
                std::ptr::null(),
                flags,
                std::ptr::null(),
                std::ptr::null(),
            )
        };
        if ok == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    fn replace_once(src: &Path, dest: &Path) -> ReplaceOutcome {
        if !dest.exists() {
            return match move_file(src, dest) {
                Ok(()) => ReplaceOutcome::Replaced,
                Err(e) => ReplaceOutcome::SourceIntact(e),
            };
        }

        let err = match replace_file_w(dest, src, 0) {
            Ok(()) => return ReplaceOutcome::Replaced,
            Err(e) => e,
        };

        match err.raw_os_error() {
            Some(ERROR_FILE_NOT_FOUND) | Some(ERROR_PATH_NOT_FOUND) => match move_file(src, dest) {
                Ok(()) => ReplaceOutcome::Replaced,
                Err(e) => ReplaceOutcome::SourceIntact(e),
            },
            // A failed ACL/attribute merge also reports 1176. Retry once with
            // the merge errors ignored (logged) rather than leaving the file
            // permanently unsaveable.
            Some(ERROR_UNABLE_TO_MOVE_REPLACEMENT) => {
                log::warn!(
                    "[fs] metadata merge failed for {}, retrying without merge: {err}",
                    dest.display()
                );
                match replace_file_w(
                    dest,
                    src,
                    REPLACEFILE_IGNORE_MERGE_ERRORS | REPLACEFILE_IGNORE_ACL_ERRORS,
                ) {
                    Ok(()) => ReplaceOutcome::Replaced,
                    Err(retry_err) => ReplaceOutcome::SourceIntact(retry_err),
                }
            }
            Some(code) if is_transient(code) => ReplaceOutcome::SourceIntact(err),
            // 1175/1177 (original already removed) and anything unknown is
            // ambiguous: guessing "intact" would let the caller delete the
            // only complete copy.
            _ => ReplaceOutcome::Indeterminate(err),
        }
    }

    pub(super) fn replace(src: &Path, dest: &Path) -> ReplaceOutcome {
        let mut last = io::Error::other("replace_file: retries exhausted");
        for attempt in 0..TRANSIENT_RETRIES {
            match replace_once(src, dest) {
                ReplaceOutcome::SourceIntact(e) if is_transient(e.raw_os_error().unwrap_or(0)) => {
                    last = e;
                    let backoff = (5 * u64::from(attempt + 1)).min(50);
                    std::thread::sleep(Duration::from_millis(backoff));
                }
                other => return other,
            }
        }
        ReplaceOutcome::SourceIntact(last)
    }
}

#[cfg(all(test, windows))]
/// Test hook: build the wide path the Windows wrappers hand to Win32.
fn windows_wide(path: &str) -> Vec<u16> {
    windows::wide(std::path::Path::new(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, bytes: &[u8]) {
        std::fs::write(path, bytes).unwrap();
    }

    fn temp_for(dir: &Path, name: &str) -> std::path::PathBuf {
        dir.join(format!("{name}.pixiv-manager-tmp-1"))
    }

    #[test]
    fn replaces_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("state.json");
        write(&dest, b"old");
        let tmp = temp_for(dir.path(), "state.json");
        write(&tmp, b"new");
        assert!(matches!(
            replace_file(&tmp, &dest),
            ReplaceOutcome::Replaced
        ));
        assert_eq!(std::fs::read(&dest).unwrap(), b"new");
        assert!(!tmp.exists());
    }

    #[test]
    fn creates_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("fresh.toml");
        let tmp = temp_for(dir.path(), "fresh.toml");
        write(&tmp, b"hello");
        assert!(matches!(
            replace_file(&tmp, &dest),
            ReplaceOutcome::Replaced
        ));
        assert_eq!(std::fs::read(&dest).unwrap(), b"hello");
    }

    #[test]
    fn replaces_repeatedly() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("colors.json");
        for i in 0..8u8 {
            let tmp = dir
                .path()
                .join(format!("colors.json.pixiv-manager-tmp-{i}"));
            write(&tmp, &[i; 3]);
            assert!(matches!(
                replace_file(&tmp, &dest),
                ReplaceOutcome::Replaced
            ));
            assert_eq!(std::fs::read(&dest).unwrap(), [i; 3]);
        }
    }

    #[test]
    fn missing_source_leaves_destination_intact() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("dest.bin");
        write(&dest, b"keep me");
        let outcome = replace_file(&dir.path().join("nope.tmp"), &dest);
        assert!(
            matches!(outcome, ReplaceOutcome::SourceIntact(_)),
            "expected SourceIntact, got {outcome:?}"
        );
        assert_eq!(std::fs::read(&dest).unwrap(), b"keep me");
    }

    #[cfg(unix)]
    #[test]
    fn read_only_directory_is_source_intact() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("locked");
        std::fs::create_dir(&sub).unwrap();
        let dest = sub.join("state.json");
        write(&dest, b"old");
        let tmp = temp_for(dir.path(), "state.json");
        write(&tmp, b"new");
        let mode = std::fs::metadata(&sub).unwrap().permissions().mode();
        std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o500)).unwrap();
        let outcome = replace_file(&tmp, &dest);
        std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(mode)).unwrap();
        assert!(
            matches!(outcome, ReplaceOutcome::SourceIntact(_)),
            "permission failure must be reported as intact: {outcome:?}"
        );
        assert!(tmp.exists());
        assert_eq!(std::fs::read(&dest).unwrap(), b"old");
    }

    #[cfg(windows)]
    #[test]
    fn read_only_destination_is_source_intact() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("state.json");
        write(&dest, b"old");
        let mut perms = std::fs::metadata(&dest).unwrap().permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(&dest, perms).unwrap();
        let tmp = temp_for(dir.path(), "state.json");
        write(&tmp, b"new");

        let outcome = replace_file(&tmp, &dest);
        let mut perms = std::fs::metadata(&dest).unwrap().permissions();
        perms.set_readonly(false);
        std::fs::set_permissions(&dest, perms).unwrap();

        assert!(
            matches!(outcome, ReplaceOutcome::SourceIntact(_)),
            "read-only destination must be reported as intact: {outcome:?}"
        );
        assert!(tmp.exists());
        assert_eq!(std::fs::read(&dest).unwrap(), b"old");
    }

    /// A handle opened without `FILE_SHARE_DELETE` is what scanners do; the
    /// destination must survive and the replacement file must not be lost.
    #[cfg(windows)]
    #[test]
    fn destination_held_open_is_source_intact() {
        use std::os::windows::fs::OpenOptionsExt;

        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("state.json");
        write(&dest, b"old");
        let tmp = temp_for(dir.path(), "state.json");
        write(&tmp, b"new");

        let holder = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&dest)
            .unwrap();
        let outcome = replace_file(&tmp, &dest);
        drop(holder);

        assert!(
            matches!(outcome, ReplaceOutcome::SourceIntact(_)),
            "sharing violation must be reported as intact: {outcome:?}"
        );
        assert!(tmp.exists());
        assert_eq!(std::fs::read(&dest).unwrap(), b"old");
    }

    #[cfg(windows)]
    #[test]
    fn short_paths_are_passed_through() {
        let wide = super::windows_wide(r"C:\tmp\state.json");
        assert_eq!(wide[0], u16::from(b'C'));
        assert_eq!(*wide.last().unwrap(), 0u16);
    }

    #[cfg(windows)]
    #[test]
    fn long_paths_get_the_verbatim_prefix() {
        let long = format!(r"C:\dir\{}", "a".repeat(300));
        let wide = super::windows_wide(&long);
        assert_eq!(
            &wide[..4],
            &[
                u16::from(b'\\'),
                u16::from(b'\\'),
                u16::from(b'?'),
                u16::from(b'\\')
            ]
        );
    }

    #[cfg(windows)]
    #[test]
    fn long_unc_paths_use_the_unc_form() {
        let long = format!(r"\\server\share\{}", "a".repeat(300));
        let wide = super::windows_wide(&long);
        let expected: Vec<u16> = r"\\?\UNC\".encode_utf16().collect();
        assert_eq!(&wide[..expected.len()], &expected[..]);
    }

    #[cfg(windows)]
    #[test]
    fn verbatim_paths_are_not_prefixed_twice() {
        let already = format!(r"\\?\C:\dir\{}", "a".repeat(300));
        let wide = super::windows_wide(&already);
        assert!(!wide.starts_with(&[
            u16::from(b'\\'),
            u16::from(b'\\'),
            u16::from(b'?'),
            u16::from(b'\\'),
            u16::from(b'\\')
        ]));
    }

    #[cfg(windows)]
    #[test]
    fn dot_segments_are_resolved_before_prefixing() {
        let long = format!(r"C:\dir\sub\..\{}", "a".repeat(300));
        let wide = super::windows_wide(&long);
        let dotdot = [u16::from(b'.'), u16::from(b'.')];
        assert!(
            !wide.windows(2).any(|pair| pair == dotdot),
            "`..` must not survive into a verbatim path"
        );
    }

    #[test]
    fn move_file_replaces_destination() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("a.bin");
        let src = dir.path().join("b.bin");
        write(&dest, b"old");
        write(&src, b"new");
        move_file(&src, &dest).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"new");
        assert!(!src.exists());
    }
}

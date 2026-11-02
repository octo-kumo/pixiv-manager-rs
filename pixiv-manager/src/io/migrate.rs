//! One-time startup migrations between on-disk layouts.
//!
//! Persistent state (library, queue, colours, metadata, viewer transforms) is
//! moved with copy-verify-delete semantics: a file disappears from the old
//! location only after its new copy has the same length. Anything that cannot
//! be moved stays where it is and is logged, so a failed migration can never
//! lose bookmarks. The image cache is disposable and migrated best effort.

use std::path::Path;

use crate::io::paths::AppPaths;

/// Age above which a temp file is treated as a crash leftover.
const STALE_TEMP_AGE: std::time::Duration = std::time::Duration::from_secs(3600);

/// Run every startup migration and cleanup, in order.
///
/// Must run before anything reads the library or the cache, and after the
/// paths are known.
pub fn run_startup_migrations(paths: &AppPaths) {
    for legacy in paths.legacy_data_roots() {
        migrate_legacy_data(&legacy, &paths.data_dir);
    }
    if let Some(legacy_cache) = crate::io::paths::legacy_image_cache_dir() {
        if legacy_cache != paths.image_cache_dir() {
            crate::image::cache::migrate_legacy_cache(&legacy_cache, &paths.image_cache_dir());
        }
    }
    for dir in [
        paths.config_dir.clone(),
        paths.library_dir(),
        paths.meta_dir(),
        paths.image_cache_dir(),
        paths.image_cache_dir().join("thumb"),
    ] {
        crate::io::cleanup_stale_atomic_temps(&dir, STALE_TEMP_AGE);
    }
}

/// Move `legacy_root/library` into `data_dir/library` when the destination is
/// missing.
///
/// Returns the number of files moved.
pub fn migrate_legacy_data(legacy_root: &Path, data_dir: &Path) -> usize {
    let from = legacy_root.join("library");
    let to = data_dir.join("library");
    if !from.is_dir() || to.exists() {
        return 0;
    }
    if std::fs::create_dir_all(data_dir).is_err() {
        log::warn!("[migrate] cannot create {}", data_dir.display());
        return 0;
    }
    match std::fs::rename(&from, &to) {
        Ok(()) => {
            log::info!("[migrate] moved {} to {}", from.display(), to.display());
            count_files(&to)
        }
        Err(rename_err) => {
            log::info!(
                "[migrate] rename failed ({rename_err}); copying {} to {}",
                from.display(),
                to.display()
            );
            copy_tree(&from, &to)
        }
    }
}

/// Copy every file, verifying its length, and only then remove the source.
fn copy_tree(from: &Path, to: &Path) -> usize {
    let mut moved = 0;
    let mut stack = vec![from.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let src = entry.path();
            let Ok(rel) = src.strip_prefix(from) else {
                continue;
            };
            let dst = to.join(rel);
            if src.is_dir() {
                let _ = std::fs::create_dir_all(&dst);
                stack.push(src);
                continue;
            }
            if dst.exists() {
                continue;
            }
            if let Some(parent) = dst.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            match std::fs::copy(&src, &dst) {
                Ok(copied) => {
                    let same_len = std::fs::metadata(&src)
                        .map(|m| m.len() == copied)
                        .unwrap_or(false);
                    if same_len {
                        let _ = std::fs::remove_file(&src);
                        moved += 1;
                    } else {
                        log::warn!("[migrate] short copy for {}", src.display());
                    }
                }
                Err(e) => log::warn!("[migrate] cannot copy {}: {e}", src.display()),
            }
        }
    }
    moved
}

fn count_files(root: &Path) -> usize {
    let mut count = 0;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            if entry.path().is_dir() {
                stack.push(entry.path());
            } else {
                count += 1;
            }
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moves_a_legacy_library_directory() {
        let dir = tempfile::tempdir().unwrap();
        let legacy = dir.path().join("pixiv-manager");
        std::fs::create_dir_all(legacy.join("library/meta")).unwrap();
        std::fs::write(legacy.join("library/bookmarks.json"), b"{}").unwrap();
        std::fs::write(legacy.join("library/meta/1.json"), b"{}").unwrap();

        let data = legacy.join("data");
        assert_eq!(migrate_legacy_data(&legacy, &data), 2);
        assert!(data.join("library/bookmarks.json").is_file());
        assert!(data.join("library/meta/1.json").is_file());
        assert!(!legacy.join("library").exists());
    }

    #[test]
    fn does_not_overwrite_an_existing_library() {
        let dir = tempfile::tempdir().unwrap();
        let legacy = dir.path().join("pixiv-manager");
        std::fs::create_dir_all(legacy.join("library")).unwrap();
        std::fs::write(legacy.join("library/bookmarks.json"), b"old").unwrap();
        let data = legacy.join("data");
        std::fs::create_dir_all(data.join("library")).unwrap();
        std::fs::write(data.join("library/bookmarks.json"), b"new").unwrap();

        assert_eq!(migrate_legacy_data(&legacy, &data), 0);
        assert_eq!(
            std::fs::read(data.join("library/bookmarks.json")).unwrap(),
            b"new"
        );
        assert_eq!(
            std::fs::read(legacy.join("library/bookmarks.json")).unwrap(),
            b"old"
        );
    }

    #[test]
    fn copy_fallback_keeps_source_until_verified() {
        let dir = tempfile::tempdir().unwrap();
        let from = dir.path().join("from");
        let to = dir.path().join("to");
        std::fs::create_dir_all(from.join("nested")).unwrap();
        std::fs::write(from.join("a.json"), b"aaaa").unwrap();
        std::fs::write(from.join("nested/b.json"), b"bbbb").unwrap();

        assert_eq!(copy_tree(&from, &to), 2);
        assert_eq!(std::fs::read(to.join("a.json")).unwrap(), b"aaaa");
        assert_eq!(std::fs::read(to.join("nested/b.json")).unwrap(), b"bbbb");
        assert!(!from.join("a.json").exists());
    }
}

use lru::LruCache;
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::sync::Mutex;

/// Only temp files older than this are removed: a younger one may belong to a
/// concurrent writer in another process.
const STALE_TEMP_AGE: std::time::Duration = std::time::Duration::from_secs(3600);

/// Default in-memory budget for cached image bytes (96 MB).
pub const DEFAULT_MEMORY_BUDGET: usize = 96 * 1024 * 1024;

/// Two-layer image cache: an LRU of raw bytes in RAM plus files keyed by
/// `sha256(url)` on disk. Corrupt disk entries are deleted and treated as a
/// miss. The app shares one instance behind an `Arc`.
pub struct ImageCache {
    disk_dir: PathBuf,
    memory: Mutex<LruCache<String, Vec<u8>>>,
    memory_bytes: Mutex<usize>,
    max_memory_bytes: usize,
}

impl Default for ImageCache {
    /// An in-memory-only cache (no disk layer). Useful for defaults/tests.
    fn default() -> Self {
        Self {
            disk_dir: PathBuf::new(),
            memory: Mutex::new(LruCache::unbounded()),
            memory_bytes: Mutex::new(0),
            max_memory_bytes: DEFAULT_MEMORY_BUDGET,
        }
    }
}

impl ImageCache {
    /// Create a cache backed by `disk_dir` with the given RAM byte budget.
    pub fn new(disk_dir: PathBuf, max_memory_bytes: usize) -> Self {
        let _ = std::fs::create_dir_all(&disk_dir);
        // Only crash leftovers: an age guard keeps a live writer's temp file
        // safe when two processes share the cache.
        crate::io::cleanup_stale_atomic_temps(&disk_dir, STALE_TEMP_AGE);
        Self {
            disk_dir,
            memory: Mutex::new(LruCache::unbounded()),
            memory_bytes: Mutex::new(0),
            max_memory_bytes,
        }
    }

    fn cache_key(url: &str) -> String {
        Sha256::digest(url.as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    fn disk_path(&self, url: &str) -> PathBuf {
        self.disk_dir.join(Self::cache_key(url))
    }

    /// Look up cached bytes for `url`: RAM layer, then disk layer. A disk hit
    /// is validated (image magic bytes) and promoted into the RAM layer.
    /// Corrupt disk entries are deleted and treated as a miss.
    pub fn get(&self, url: &str) -> Option<Vec<u8>> {
        let key = Self::cache_key(url);
        {
            let mut memory = self.memory.lock().unwrap();
            if let Some(data) = memory.get(&key) {
                return Some(data.clone());
            }
        }

        let path = self.disk_path(url);
        if path.exists() {
            #[cfg(feature = "profiling")]
            let start = std::time::Instant::now();
            if let Ok(data) = std::fs::read(&path) {
                #[cfg(feature = "profiling")]
                crate::profiling::tracing::record_file(
                    crate::profiling::tracing::FileOp::Read,
                    path.display().to_string(),
                    data.len(),
                    start.elapsed(),
                );
                if looks_like_image(&data) {
                    self.insert_memory(key, data.clone());
                    return Some(data);
                }
                log::warn!("[image_cache] corrupt entry, deleting {:?}", path);
                let _ = std::fs::remove_file(&path);
            }
        }
        None
    }

    /// Insert bytes into the RAM layer (respecting the byte budget) and write
    /// them to disk atomically. No-op for the RAM-only default cache.
    pub fn insert(&self, url: &str, bytes: Vec<u8>) {
        let key = Self::cache_key(url);
        self.insert_memory(key.clone(), bytes.clone());
        if self.disk_dir.as_os_str().is_empty() {
            return;
        }
        write_atomic(&self.disk_dir, &key, &bytes);
    }

    /// Like [`ImageCache::insert`] but performs the disk write on a blocking
    /// thread so a large image doesn't stall the UI thread. The RAM layer is
    /// updated synchronously.
    pub async fn insert_async(&self, url: &str, bytes: Vec<u8>) {
        let key = Self::cache_key(url);
        self.insert_memory(key.clone(), bytes.clone());
        if self.disk_dir.as_os_str().is_empty() {
            return;
        }
        let dir = self.disk_dir.clone();
        tokio::task::spawn_blocking(move || write_atomic(&dir, &key, &bytes))
            .await
            .ok();
    }

    fn insert_memory(&self, key: String, bytes: Vec<u8>) {
        let mut memory = self.memory.lock().unwrap();
        let mut bytes_total = self.memory_bytes.lock().unwrap();
        if let Some(old) = memory.pop(&key) {
            *bytes_total = bytes_total.saturating_sub(old.len());
        }
        *bytes_total += bytes.len();
        memory.put(key, bytes);
        // Evict least-recently-used entries until within the byte budget.
        while *bytes_total > self.max_memory_bytes {
            if let Some((_, evicted)) = memory.pop_lru() {
                *bytes_total = bytes_total.saturating_sub(evicted.len());
            } else {
                break;
            }
        }
    }

    /// Current in-memory cache size in bytes (for the debug overlay).
    pub fn memory_bytes(&self) -> usize {
        *self.memory_bytes.lock().unwrap()
    }

    /// Drop every cached entry (RAM + disk). Used to invalidate the
    /// *processed* thumbnail cache when the grid size or quality option
    /// changes — the raw URL→bytes cache is a separate instance and survives.
    pub fn clear(&self) {
        let mut memory = self.memory.lock().unwrap();
        memory.clear();
        *self.memory_bytes.lock().unwrap() = 0;
        if let Ok(entries) = std::fs::read_dir(&self.disk_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                let name = entry.file_name();
                if !crate::io::is_atomic_temp_file(&name) {
                    let _ = std::fs::remove_file(path);
                }
            }
        }
    }
}

/// Write `bytes` to `dir/key` through the shared atomic writer.
fn write_atomic(dir: &std::path::Path, key: &str, bytes: &[u8]) {
    let final_path = dir.join(key);
    if let Err(e) = crate::io::write_atomic(&final_path, bytes) {
        log::warn!("[image_cache] disk write failed: {e}");
    }
}

/// Best-effort move of a legacy cache tree into the current cache directory.
///
/// Recursive because the cache has namespaces (thumbnails live in `thumb/`).
/// The cache is disposable: failures are logged and the legacy tree is left
/// in place rather than risking partial deletion of anything else.
pub fn migrate_legacy_cache(legacy: &std::path::Path, current: &std::path::Path) {
    if !legacy.is_dir() || legacy == current {
        return;
    }
    let moved = migrate_tree(legacy, current);
    log::info!(
        "[image_cache] migrated {moved} cached file(s) from {} to {}",
        legacy.display(),
        current.display()
    );
}

fn migrate_tree(from_root: &std::path::Path, to_root: &std::path::Path) -> usize {
    let mut moved = 0;
    let mut dirs = vec![std::path::PathBuf::new()];
    while let Some(rel) = dirs.pop() {
        let from_dir = from_root.join(&rel);
        let Ok(entries) = std::fs::read_dir(&from_dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let rel_child = rel.join(entry.file_name());
            let from = from_root.join(&rel_child);
            let to = to_root.join(&rel_child);
            if from.is_dir() {
                if std::fs::create_dir_all(&to).is_ok() {
                    dirs.push(rel_child);
                }
                continue;
            }
            if !from.is_file() || to.exists() {
                continue;
            }
            if let Some(parent) = to.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let ok = std::fs::rename(&from, &to).is_ok()
                || (std::fs::copy(&from, &to).is_ok() && std::fs::remove_file(&from).is_ok());
            if ok {
                moved += 1;
            }
        }
    }
    moved
}

/// Cheap magic-byte check for the image formats Pixiv serves (JPEG/PNG/GIF/WebP).
fn looks_like_image(bytes: &[u8]) -> bool {
    if bytes.len() < 12 {
        return false;
    }
    matches!(
        bytes,
        [0xFF, 0xD8, 0xFF, ..]                       // JPEG
            | [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, ..] // PNG
            | [b'G', b'I', b'F', b'8', ..]           // GIF
            | [b'R', b'I', b'F', b'F', .., b'W', b'E', b'B', b'P'] // WebP
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_key_is_deterministic() {
        let k1 = ImageCache::cache_key("https://example.com/1.jpg");
        let k2 = ImageCache::cache_key("https://example.com/1.jpg");
        assert_eq!(k1, k2);
    }

    #[test]
    fn cache_key_differs_for_different_urls() {
        let k1 = ImageCache::cache_key("https://example.com/1.jpg");
        let k2 = ImageCache::cache_key("https://example.com/2.jpg");
        assert_ne!(k1, k2);
    }

    #[test]
    fn memory_get_insert_roundtrip() {
        let cache = ImageCache::default();
        assert!(cache.get("url1").is_none());
        cache.insert("url1", vec![1, 2, 3]);
        assert_eq!(cache.get("url1"), Some(vec![1, 2, 3]));
    }

    #[test]
    fn memory_lru_eviction() {
        let cache = ImageCache::new(PathBuf::new(), 10);
        cache.insert("k1", vec![0; 6]);
        cache.insert("k2", vec![0; 6]); // budget 10 → evicts k1
        assert!(cache.get("k1").is_none());
        assert!(cache.get("k2").is_some());
    }

    #[test]
    fn disk_persists_and_is_read_back() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        let cache = ImageCache::new(dir.clone(), 1000);
        cache.insert(
            "https://example.com/x.jpg",
            vec![0xFF, 0xD8, 0xFF, 0xE0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 2, 3],
        );

        // A fresh instance (empty RAM) must find it on disk.
        let reloaded = ImageCache::new(dir, 1000);
        assert_eq!(
            reloaded.get("https://example.com/x.jpg"),
            Some(vec![
                0xFF, 0xD8, 0xFF, 0xE0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 2, 3
            ])
        );
    }

    #[test]
    fn corrupt_disk_entry_is_deleted_and_treated_as_miss() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        let cache = ImageCache::new(dir.clone(), 1000);
        cache.insert(
            "https://example.com/bad.jpg",
            vec![0xFF, 0xD8, 0xFF, 0xE0, 0, 0, 0, 0, 0, 0, 0, 0, 9, 9, 9],
        );

        // Corrupt the stored file, then read through a fresh instance so the
        // RAM layer doesn't shadow the disk.
        let path = dir.join(ImageCache::cache_key("https://example.com/bad.jpg"));
        std::fs::write(&path, b"not an image").unwrap();
        let reloaded = ImageCache::new(dir, 1000);

        assert!(reloaded.get("https://example.com/bad.jpg").is_none());
        assert!(!path.exists(), "corrupt entry should be deleted");
    }

    #[test]
    fn non_image_bytes_rejected() {
        assert!(!looks_like_image(b"this is definitely not an image"));
        assert!(!looks_like_image(b""));
        assert!(!looks_like_image(&[0xFF]));
        assert!(looks_like_image(&[
            0xFF, 0xD8, 0xFF, 0xE0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0
        ]));
        assert!(looks_like_image(&[
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 0, 0, 0, 0, 0
        ]));
    }
}

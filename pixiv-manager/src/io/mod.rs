pub mod colors;
pub mod config;
pub mod download;
pub mod library;
pub mod meta_cache;

use std::path::Path;

/// Read a file to a string with optional profiling instrumentation.
pub(crate) fn read_file(path: &Path) -> Option<String> {
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
pub(crate) fn save_json_atomic<T: serde::Serialize>(value: &T, path: &Path) -> std::io::Result<()> {
    let content = serde_json::to_string_pretty(value)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    write_atomic(path, content.as_bytes())
}

/// Write `bytes` to `path` atomically (temp file + rename).
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    #[cfg(feature = "profiling")]
    let start = std::time::Instant::now();
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    #[cfg(feature = "profiling")]
    crate::profiling::tracing::record_file(
        crate::profiling::tracing::FileOp::Write,
        path.display().to_string(),
        bytes.len(),
        start.elapsed(),
    );
    match std::fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        // Windows refuses to overwrite; replace.
        Err(_) if path.exists() => {
            std::fs::remove_file(path)?;
            std::fs::rename(&tmp, path)
        }
        Err(e) => Err(e),
    }
}

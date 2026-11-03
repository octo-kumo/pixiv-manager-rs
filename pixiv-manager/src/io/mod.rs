//! Application persistence.
//!
//! All durable writes go through [`atomic`]; no module implements its own
//! temp-file-plus-rename routine.

pub mod atomic;
pub mod colors;
pub mod config;
pub mod download;
pub mod library;
pub mod meta_cache;
pub mod migrate;
pub mod paths;

pub use atomic::{
    cleanup_stale_atomic_temps, is_atomic_temp_file, save_json_atomic, save_json_compact_atomic,
    write_atomic, write_atomic_private, AtomicWriteError, TEMP_MARKER,
};

use std::path::Path;

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

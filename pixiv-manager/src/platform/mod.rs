//! Platform-specific primitives used by otherwise portable application code.
//!
//! Application logic must not contain `cfg(target_os)` blocks; differences
//! between operating systems belong here (or in `io::paths`).

pub mod fonts;
pub mod fs;
pub mod memory;
pub mod shortcuts;

pub use fs::replace_file;
pub use memory::{memory_stats, MemoryStats};
pub use shortcuts::{primary_label, primary_modifier, primary_shortcut};

pub fn os_name() -> &'static str {
    std::env::consts::OS
}

pub fn arch() -> &'static str {
    std::env::consts::ARCH
}

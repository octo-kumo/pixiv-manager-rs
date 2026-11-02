//! Narrow OS abstraction layer.
//!
//! Everything the application is allowed to know about the *host operating
//! system* lives here (directory layout lives in [`crate::io::paths`]). The
//! reducers, widgets, provider clients, downloader and persistence modules
//! must never contain `cfg(target_os = …)` blocks — they call this module
//! instead, so adding another desktop OS means adding an implementation here
//! rather than touching application logic.
//!
//! Contents:
//!
//! * [`memory`] — process/system memory figures for the debug overlay.
//! * [`shortcuts`] — the platform's primary shortcut modifier (Ctrl / ⌘) and
//!   the labels derived from it.
//! * [`fs`] — the one atomic file-replacement primitive used by every
//!   persistent writer in the application.

pub mod fs;
pub mod memory;
pub mod shortcuts;

pub use fs::replace_file;
pub use memory::{memory_stats, MemoryStats};
pub use shortcuts::{primary_label, primary_modifier, primary_shortcut};

/// The operating system name shown in the debug overlay (`linux`, `windows`,
/// `macos`).
pub fn os_name() -> &'static str {
    std::env::consts::OS
}

/// The CPU architecture shown in the debug overlay (`x86_64`, `aarch64`, …).
pub fn arch() -> &'static str {
    std::env::consts::ARCH
}

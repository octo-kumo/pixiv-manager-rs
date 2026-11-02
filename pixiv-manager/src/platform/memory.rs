//! Process/system memory figures for the F3 debug overlay.
//!
//! Telemetry is *best effort*: every figure is an `Option`, and unsupported
//! platforms simply report `None` (the overlay renders `N/A`). Debug
//! telemetry must never become a portability blocker, which is why the
//! Linux-only `/proc` reads used to live in `app::view` were replaced by
//! [`sysinfo`]: one implementation, no `/proc` paths outside this module, and
//! the same figures on Linux, Windows and macOS.

use std::sync::{Mutex, OnceLock};

/// Memory figures reported by [`memory_stats`]. `None` means "not available
/// on this platform", which the UI renders as `N/A`.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct MemoryStats {
    /// Resident memory of *this* process, in MiB.
    pub process_used_mb: Option<f64>,
    /// Total physical memory of the machine, in MiB.
    pub system_total_mb: Option<f64>,
}

/// Cached probe so the overlay's per-frame refresh stays cheap: creating a
/// `System` enumerates processes, refreshing one pid does not.
struct Probe {
    system: sysinfo::System,
    pid: sysinfo::Pid,
}

fn probe() -> Option<&'static Mutex<Probe>> {
    static PROBE: OnceLock<Option<Mutex<Probe>>> = OnceLock::new();
    PROBE
        .get_or_init(|| {
            let pid = sysinfo::get_current_pid().ok()?;
            let system =
                sysinfo::System::new_with_specifics(sysinfo::RefreshKind::nothing()).ok()?;
            Some(Mutex::new(Probe { system, pid }))
        })
        .as_ref()
}

const BYTES_PER_MIB: f64 = 1024.0 * 1024.0;

/// Snapshot the current process RSS and the machine's total RAM.
///
/// Never panics and never blocks on I/O beyond the OS counters themselves; a
/// poisoned lock or a platform error yields a `None` figure.
pub fn memory_stats() -> MemoryStats {
    let Some(probe) = probe() else {
        return MemoryStats::default();
    };
    let Ok(mut probe) = probe.lock() else {
        return MemoryStats::default();
    };
    let Probe { system, pid } = &mut *probe;
    system.refresh_memory();
    // Only this process: enumerating every pid on the machine would make the
    // per-frame overlay expensive.
    system.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[*pid]), false);
    let process_used_mb = system
        .process(*pid)
        .map(|process| process.memory() as f64 / BYTES_PER_MIB);
    let total = system.total_memory();
    MemoryStats {
        process_used_mb,
        system_total_mb: (total > 0).then_some(total as f64 / BYTES_PER_MIB),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every supported desktop target must produce real figures: the overlay
    /// is not decorative — it is how Windows/macOS users diagnose the same
    /// cache/queue problems Linux users do with htop.
    #[test]
    fn reports_process_and_system_memory() {
        let stats = memory_stats();
        let used = stats.process_used_mb.expect("process RSS");
        let total = stats.system_total_mb.expect("total system memory");
        assert!(used > 0.0, "process RSS should be non-zero: {used}");
        assert!(
            used < total,
            "process RSS ({used} MiB) should be below total RAM ({total} MiB)"
        );
        assert!(total > 16.0, "implausible total RAM: {total} MiB");
    }

    /// Repeated calls must stay stable (the probe is cached, and refreshing a
    /// single pid must not reset the totals).
    #[test]
    fn repeated_calls_are_consistent() {
        let a = memory_stats();
        let b = memory_stats();
        assert!(a.system_total_mb.is_some());
        assert_eq!(
            a.system_total_mb.map(|f| f.round() as u64),
            b.system_total_mb.map(|f| f.round() as u64)
        );
    }
}

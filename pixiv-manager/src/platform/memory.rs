//! Best-effort process and system memory figures.
//!
//! Values the platform cannot report are `None`; the debug overlay renders
//! them as `N/A`.

use std::sync::{Mutex, OnceLock};

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct MemoryStats {
    pub process_used_mb: Option<f64>,
    pub system_total_mb: Option<f64>,
}

const BYTES_PER_MIB: f64 = 1024.0 * 1024.0;

/// Cached so the per-frame overlay refresh only re-reads the counters instead
/// of enumerating every process on the machine.
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

pub fn memory_stats() -> MemoryStats {
    let Some(probe) = probe() else {
        return MemoryStats::default();
    };
    let Ok(mut probe) = probe.lock() else {
        return MemoryStats::default();
    };
    let Probe { system, pid } = &mut *probe;
    system.refresh_memory();
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

    #[test]
    fn reports_plausible_figures() {
        let stats = memory_stats();
        let used = stats.process_used_mb.expect("process RSS");
        let total = stats.system_total_mb.expect("total memory");
        assert!(used > 0.0 && used < total, "used={used} total={total}");
        assert!(total > 16.0, "implausible total: {total}");
    }

    #[test]
    fn repeated_calls_agree_on_total() {
        let a = memory_stats().system_total_mb.map(|v| v.round() as u64);
        let b = memory_stats().system_total_mb.map(|v| v.round() as u64);
        assert!(a.is_some());
        assert_eq!(a, b);
    }
}

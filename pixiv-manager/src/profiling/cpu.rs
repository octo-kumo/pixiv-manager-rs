//! CPU sampling via `pprof-rs` (feature = `profiling`).
//!
//! A sampling profiler whose capture can be started/stopped directly
//! ([`start`]/[`stop`]) or over HTTP (see [`super::http`]), producing a
//! flamegraph SVG plus a text hotspots table. Symbol resolution uses debug
//! info (build with `--profile profiling`) for line-level culprits.

use std::collections::BTreeMap;
use std::sync::Mutex;

static GUARD: Mutex<Option<pprof::ProfilerGuard<'static>>> = Mutex::new(None);
static FREQUENCY: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(100);

/// Set the sampling frequency in Hz (default 100).
pub fn set_frequency(hz: i32) {
    FREQUENCY.store(hz, std::sync::atomic::Ordering::Relaxed);
}

/// Start sampling (idempotent).
pub fn start() -> Result<(), String> {
    let mut guard = GUARD.lock().unwrap();
    if guard.is_none() {
        let builder = pprof::ProfilerGuardBuilder::default()
            .frequency(FREQUENCY.load(std::sync::atomic::Ordering::Relaxed));
        *guard = Some(builder.build().map_err(|e| e.to_string())?);
    }
    Ok(())
}

/// Stop sampling and return the report (symbols resolved), if a capture was
/// running.
pub fn stop() -> Option<pprof::Report> {
    let mut guard = GUARD.lock().unwrap();
    guard
        .take()
        .map(|g| g.report().build().expect("pprof report build failed"))
}

/// Write a flamegraph SVG to `writer`.
pub fn write_flamegraph(
    report: &pprof::Report,
    writer: &mut dyn std::io::Write,
) -> Result<(), pprof::Error> {
    report.flamegraph(writer)
}

/// A hotspot: total sample count and self sample count for one function.
#[derive(Debug, Clone)]
pub struct Hotspot {
    pub function: String,
    pub location: String,
    pub self_samples: i64,
    pub total_samples: i64,
}

/// Kernel/libc/threading shims that dominate idle-thread samples; skipped for
/// **self** attribution so the table shows real application work.
fn is_idle_shim(name: &str) -> bool {
    matches!(
        name,
        "_start"
            | "__libc_start_main"
            | "__clone3"
            | "clone"
            | "clone3"
            | "fork"
            | "vfork"
            | "epoll_wait"
            | "epoll_pwait"
            | "epoll_pwait2"
            | "futex"
            | "syscall"
            | "nanosleep"
            | "clock_nanosleep"
            | "__futex_wait"
    ) || name.ends_with(".park")
        || name.contains("futex")
}

/// Build a text table of the top functions by **self** time, with source
/// locations. `frequency_hz` converts sample counts to approximate seconds.
pub fn hotspots(report: &pprof::Report, frequency_hz: i32) -> Vec<Hotspot> {
    let mut self_map: BTreeMap<(String, String), i64> = BTreeMap::new();
    let mut total_map: BTreeMap<(String, String), i64> = BTreeMap::new();

    for (frames, count) in &report.data {
        let count = *count;
        // Backtraces are ordered root→leaf, so the leaf (the function that
        // was actually running) is the FIRST symbol group.
        if let Some(first_vec) = frames.frames.first() {
            if let Some(leaf) = first_vec.last() {
                let key = (
                    leaf.name(),
                    format!("{}:{}", leaf.filename(), leaf.lineno()),
                );
                if !is_idle_shim(&leaf.name()) {
                    *self_map.entry(key.clone()).or_insert(0) += count as i64;
                }
                *total_map.entry(key).or_insert(0) += count as i64;
            }
        }
        // Every frame on the stack contributes to its total time.
        for vec in &frames.frames {
            for symbol in vec {
                let key = (
                    symbol.name(),
                    format!("{}:{}", symbol.filename(), symbol.lineno()),
                );
                *total_map.entry(key).or_insert(0) += count as i64;
            }
        }
    }

    let mut out: Vec<Hotspot> = self_map
        .iter()
        .map(|((function, location), self_samples)| Hotspot {
            function: function.clone(),
            location: location.clone(),
            self_samples: *self_samples,
            total_samples: *total_map
                .get(&(function.clone(), location.clone()))
                .unwrap_or(&0),
        })
        .collect();
    out.sort_by_key(|a| std::cmp::Reverse(a.self_samples));
    let _ = frequency_hz; // kept for future per-second formatting
    out.truncate(50);
    out
}

/// Render the hotspots table as markdown.
pub fn hotspots_markdown(report: &pprof::Report, frequency_hz: i32) -> String {
    let mut md = String::from("| # | self % | self (s) | total (s) | function | location |\n");
    md.push_str("|---|--------|----------|-----------|----------|----------|\n");
    let total_samples: i64 = report
        .data
        .values()
        .map(|c| (*c).max(0) as i64)
        .sum::<i64>()
        .max(1);
    let hz = frequency_hz.max(1) as f64;
    for (i, h) in hotspots(report, frequency_hz).iter().enumerate() {
        let self_pct = h.self_samples as f64 / total_samples as f64 * 100.0;
        let self_s = h.self_samples as f64 / hz;
        let total_s = h.total_samples as f64 / hz;
        md.push_str(&format!(
            "| {} | {:.1}% | {:.3}s | {:.3}s | `{}` | `{}` |\n",
            i + 1,
            self_pct,
            self_s,
            total_s,
            h.function,
            h.location
        ));
    }
    md
}

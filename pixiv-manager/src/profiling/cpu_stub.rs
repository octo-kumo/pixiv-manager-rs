//! CPU sampling — non-Unix stub.
//!
//! `pprof-rs` only supports Unix (it samples with `perf`/`setitimer` and
//! unwinds native stacks). The `profiling` feature must still *compile* on
//! Windows, because it also carries the cross-platform pieces (the tagged
//! allocator, the HTTP control server, the report writer and the tracing
//! hooks), so this module mirrors [`super::cpu`]'s API with an engine that
//! reports "unsupported" instead of failing to build.
//!
//! What still works on Windows with `--features profiling`: allocation
//! profiling, per-message `update()`/`view()` timings, file/HTTP tracing and
//! `report.md`/`report.json`. What does not: CPU sampling, flamegraphs and
//! the hotspots table. Normal (non-`profiling`) Windows builds are unaffected
//! and never link any of this.

/// Placeholder report: CPU sampling cannot produce one on this platform.
#[derive(Debug, Clone, Copy)]
pub struct Report;

/// A hotspot table row (never produced by the stub, defined for API parity).
#[derive(Debug, Clone)]
pub struct Hotspot {
    pub function: String,
    pub location: String,
    pub self_samples: i64,
    pub total_samples: i64,
}

/// Set the sampling frequency in Hz. Ignored: no sampler exists here.
pub fn set_frequency(_hz: i32) {}

/// Start sampling. Always fails with an explanatory message.
pub fn start() -> Result<(), String> {
    Err("CPU sampling (pprof) is only available on Unix platforms".to_string())
}

/// Stop sampling. There is never a capture to stop.
pub fn stop() -> Option<Report> {
    None
}

/// Write a flamegraph SVG. Nothing can be captured, so there is nothing to
/// write.
pub fn write_flamegraph(_report: &Report, _writer: &mut dyn std::io::Write) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "CPU sampling is only available on Unix platforms",
    ))
}

/// No hotspots are available without a sampler.
pub fn hotspots(_report: &Report, _frequency_hz: i32) -> Vec<Hotspot> {
    Vec::new()
}

/// Markdown table describing the CPU hotspots; explains the limitation
/// instead of pretending there were no samples.
pub fn hotspots_markdown(_report: &Report, _frequency_hz: i32) -> String {
    "\
CPU sampling is unavailable on this platform (pprof supports Unix only).
\n\n"
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_reports_unsupported_instead_of_panicking() {
        let err = start().expect_err("no sampler on this platform");
        assert!(err.contains("Unix"), "unhelpful message: {err}");
        assert!(stop().is_none());
        assert!(hotspots_markdown(&Report, 100).contains("unavailable"));
    }
}

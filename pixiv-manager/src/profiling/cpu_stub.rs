//! CPU sampling stub for platforms without `pprof` (Unix-only).

/// Placeholder capture; this platform cannot produce one.
pub struct Report;

pub struct Hotspot {
    pub function: String,
    pub location: String,
    pub self_samples: i64,
    pub total_samples: i64,
}

pub fn set_frequency(_hz: i32) {}

pub fn start() -> Result<(), String> {
    Err("CPU sampling is only available on Unix platforms".to_string())
}

pub fn stop() -> Option<Report> {
    None
}

pub fn write_flamegraph(_report: &Report, _writer: &mut dyn std::io::Write) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "CPU sampling is only available on Unix platforms",
    ))
}

pub fn hotspots(_report: &Report, _frequency_hz: i32) -> Vec<Hotspot> {
    Vec::new()
}

pub fn hotspots_markdown(_report: &Report, _frequency_hz: i32) -> String {
    "CPU sampling is unavailable on this platform.\n".to_string()
}

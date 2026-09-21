//! Tiny HTTP control server (feature = `profiling`) so an AI agent can drive
//! profiling of the **running GUI app** with `curl` — no terminal access to
//! the process needed:
//!
//! ```text
//! GET /profile/start            start CPU sampling (optional ?freq=100)
//! GET /profile/stop             stop, write flamegraph.svg + hotspots.md
//! GET /profile/flamegraph       return the flamegraph SVG
//! GET /profile/hotspots         return the top-functions markdown table
//! GET /metrics                  return the raw JSON snapshot
//! GET /report                   write report.md/report.json, return report.md
//! ```
//!
//! Built with `std::net::TcpListener` only (no HTTP dependency), mirroring
//! the onboarding server.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::OnceLock;

static OUT_DIR: OnceLock<PathBuf> = OnceLock::new();

/// Set where `stop`/`report` write their artifacts (default `./profiles/live`).
pub fn init(out_dir: PathBuf) {
    let _ = OUT_DIR.set(out_dir);
}

fn out_dir() -> PathBuf {
    OUT_DIR
        .get()
        .cloned()
        .unwrap_or_else(|| PathBuf::from("profiles/live"))
}

/// Start the control server on `127.0.0.1:port` (0 = ephemeral). Returns the
/// bound port.
pub fn spawn(port: u16) -> std::io::Result<u16> {
    let listener = TcpListener::bind(("127.0.0.1", port))?;
    let bound = listener.local_addr()?.port();
    std::thread::Builder::new()
        .name("profiler-http".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                std::thread::spawn(|| {
                    let _ = handle(stream);
                });
            }
        })?;
    Ok(bound)
}

fn handle(mut stream: TcpStream) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request_line = String::new();
    if reader.read_line(&mut request_line)? == 0 {
        return Ok(());
    }
    // Only GET is supported; parse the path.
    let path = request_line
        .split_whitespace()
        .nth(1)
        .unwrap_or("/")
        .to_string();
    // Drain the rest of the request headers.
    for line in reader.lines() {
        let line = line?;
        if line.is_empty() {
            break;
        }
    }

    let (status, content_type, body) = route(&path);

    let mut response = format!(
        "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        status,
        content_type,
        body.len()
    );
    response.push_str(&body);
    stream.write_all(response.as_bytes())?;
    stream.flush()
}

fn route(path: &str) -> (&'static str, &'static str, String) {
    let (path, query) = match path.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (path, None),
    };

    match path {
        "/profile/start" => {
            let freq = query
                .and_then(|q| q.split('&').find_map(|kv| kv.strip_prefix("freq=")))
                .and_then(|v| v.parse::<i32>().ok())
                .unwrap_or(100);
            super::cpu::set_frequency(freq);
            match super::cpu::start() {
                Ok(()) => (
                    "200 OK",
                    "text/plain",
                    format!("profiling started at {freq}Hz\n"),
                ),
                Err(e) => ("500 Internal Server Error", "text/plain", format!("{e}\n")),
            }
        }
        "/profile/stop" => {
            let Some(report) = super::cpu::stop() else {
                return ("400 Bad Request", "text/plain", "not profiling\n".into());
            };
            let dir = out_dir();
            std::fs::create_dir_all(&dir).ok();
            let svg_path = dir.join("flamegraph.svg");
            let mut svg = Vec::new();
            let _ = super::cpu::write_flamegraph(&report, &mut svg);
            let _ = std::fs::write(&svg_path, &svg);
            let md = super::cpu::hotspots_markdown(&report, 100);
            let _ = std::fs::write(dir.join("hotspots.md"), &md);
            (
                "200 OK",
                "text/markdown",
                format!("capture saved to {}\n\n{}", svg_path.display(), md),
            )
        }
        "/profile/flamegraph" => {
            let path = out_dir().join("flamegraph.svg");
            match std::fs::read(&path) {
                Ok(bytes) => {
                    let body = String::from_utf8_lossy(&bytes).into_owned();
                    ("200 OK", "image/svg+xml", body)
                }
                Err(_) => ("404 Not Found", "text/plain", "no capture yet\n".into()),
            }
        }
        "/profile/hotspots" => {
            let path = out_dir().join("hotspots.md");
            match std::fs::read_to_string(&path) {
                Ok(md) => ("200 OK", "text/markdown", md),
                Err(_) => ("404 Not Found", "text/plain", "no capture yet\n".into()),
            }
        }
        "/metrics" => {
            let snapshot = super::tracing::snapshot();
            let json = super::report::snapshot_json(&snapshot);
            ("200 OK", "application/json", json)
        }
        "/report" => {
            let snapshot = super::tracing::snapshot();
            let mem = super::memory::snapshot();
            let (md, _json) = super::report::write(&snapshot, &mem, &out_dir());
            (
                "200 OK",
                "text/markdown",
                format!(
                    "report written to {}\n\n{}",
                    out_dir().join("report.md").display(),
                    md
                ),
            )
        }
        _ => ("404 Not Found", "text/plain", "unknown endpoint\n".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_start_stop_roundtrip() {
        init(std::env::temp_dir().join("profiler-http-test"));
        let (s, ct, _) = route("/profile/start");
        assert_eq!(s, "200 OK");
        assert_eq!(ct, "text/plain");
        // stop writes artifacts
        let (s, _, body) = route("/profile/stop");
        assert_eq!(s, "200 OK");
        assert!(body.contains("flamegraph.svg"));
    }

    #[test]
    fn route_metrics_returns_json() {
        init(std::env::temp_dir().join("profiler-http-test2"));
        let (s, ct, body) = route("/metrics");
        assert_eq!(s, "200 OK");
        assert_eq!(ct, "application/json");
        assert!(body.starts_with('{'));
    }
}

//! Lightweight global profiling tracer (feature = `profiling`).
//!
//! This is the shared sink for all profiling telemetry in the workspace:
//! HTTP requests, file I/O, per-message `update()`/`view()` timings, async
//! task timings and frame intervals.
//!
//! * Everything lives behind the `profiling` feature — when it is off this
//!   module is empty and every call site compiles to nothing (zero overhead).
//! * Recording is a single `Mutex` push of a small struct plus (for HTTP and
//!   file samples) a `#[track_caller]` source location, so the culprit line
//!   is known with no manual instrumentation.
//! * The tracer is a process-wide singleton ([`tracer()`]); the app-level
//!   report writer (`pixiv-manager::profiling::report`) drains it via
//!   [`Snapshot`].

use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// A single HTTP request sample.
#[derive(Debug, Clone)]
pub struct HttpSample {
    pub method: &'static str,
    pub url: String,
    pub status: u16,
    pub duration: Duration,
    pub bytes: usize,
    /// `(file, line)` of the call site, from `#[track_caller]`.
    pub caller: (&'static str, u32),
}

/// Kind of a file operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileOp {
    Read,
    Write,
}

/// A single file I/O sample.
#[derive(Debug, Clone)]
pub struct FileSample {
    pub op: FileOp,
    pub path: String,
    pub bytes: usize,
    pub duration: Duration,
    pub caller: (&'static str, u32),
}

/// A named phase (update message, view, async task) with its duration.
#[derive(Debug, Clone)]
pub struct PhaseSample {
    pub name: String,
    pub duration: Duration,
}

/// A consistent snapshot of everything recorded so far.
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub http: Vec<HttpSample>,
    pub file: Vec<FileSample>,
    pub update: Vec<PhaseSample>,
    pub view: Vec<PhaseSample>,
    pub task: Vec<PhaseSample>,
    pub frames: Vec<Duration>,
    pub elapsed: Duration,
}

/// Cap on the number of stored samples so a runaway recording cannot exhaust
/// memory (the allocator is itself traced). Bounds are per-category.
const MAX_SAMPLES: usize = 200_000;

struct Profiler {
    http: Mutex<Vec<HttpSample>>,
    file: Mutex<Vec<FileSample>>,
    update: Mutex<Vec<PhaseSample>>,
    view: Mutex<Vec<PhaseSample>>,
    task: Mutex<Vec<PhaseSample>>,
    frames: Mutex<Vec<Duration>>,
    start: Mutex<Instant>,
}

impl Profiler {
    fn new() -> Self {
        Self {
            http: Mutex::new(Vec::new()),
            file: Mutex::new(Vec::new()),
            update: Mutex::new(Vec::new()),
            view: Mutex::new(Vec::new()),
            task: Mutex::new(Vec::new()),
            frames: Mutex::new(Vec::new()),
            start: Mutex::new(Instant::now()),
        }
    }
}

/// The process-wide tracer singleton.
#[allow(private_interfaces)]
pub(crate) fn tracer() -> &'static Profiler {
    static TRACER: OnceLock<Profiler> = OnceLock::new();
    TRACER.get_or_init(Profiler::new)
}

fn push_capped<T>(vec: &mut Vec<T>, item: T) {
    if vec.len() < MAX_SAMPLES {
        vec.push(item);
    }
}

fn caller() -> (&'static str, u32) {
    let loc = std::panic::Location::caller();
    (loc.file(), loc.line())
}

/// Record an HTTP request (method, URL, status, duration, response bytes).
#[track_caller]
pub fn record_http(
    method: &'static str,
    url: String,
    status: u16,
    duration: Duration,
    bytes: usize,
) {
    let (file, line) = caller();
    let mut guard = tracer().http.lock().unwrap();
    push_capped(
        &mut *guard,
        HttpSample {
            method,
            url,
            status,
            duration,
            bytes,
            caller: (file, line),
        },
    );
}

/// Record a file read/write (op, path, bytes, duration).
#[track_caller]
pub fn record_file(op: FileOp, path: String, bytes: usize, duration: Duration) {
    let (file, line) = caller();
    let mut guard = tracer().file.lock().unwrap();
    push_capped(
        &mut *guard,
        FileSample {
            op,
            path,
            bytes,
            duration,
            caller: (file, line),
        },
    );
}

/// Record a named phase (typically one `update()` message) duration.
pub fn record_update(name: String, duration: Duration) {
    let mut guard = tracer().update.lock().unwrap();
    push_capped(&mut *guard, PhaseSample { name, duration });
}

/// Record a `view()` duration for the given phase (e.g. the active tab).
pub fn record_view(name: String, duration: Duration) {
    let mut guard = tracer().view.lock().unwrap();
    push_capped(&mut *guard, PhaseSample { name, duration });
}

/// Record an async task duration (e.g. `thumbnail_fetch`, `library_save`).
pub fn record_task(name: String, duration: Duration) {
    let mut guard = tracer().task.lock().unwrap();
    push_capped(&mut *guard, PhaseSample { name, duration });
}

/// Record one frame interval.
pub fn record_frame(interval: Duration) {
    let mut guard = tracer().frames.lock().unwrap();
    push_capped(&mut *guard, interval);
}

/// Forget everything recorded so far (used at the start of a session).
pub fn clear() {
    let t = tracer();
    t.http.lock().unwrap().clear();
    t.file.lock().unwrap().clear();
    t.update.lock().unwrap().clear();
    t.view.lock().unwrap().clear();
    t.task.lock().unwrap().clear();
    t.frames.lock().unwrap().clear();
    *t.start.lock().unwrap() = Instant::now();
}

/// Copy everything recorded so far into a [`Snapshot`].
pub fn snapshot() -> Snapshot {
    let t = tracer();
    Snapshot {
        http: t.http.lock().unwrap().clone(),
        file: t.file.lock().unwrap().clone(),
        update: t.update.lock().unwrap().clone(),
        view: t.view.lock().unwrap().clone(),
        task: t.task.lock().unwrap().clone(),
        frames: t.frames.lock().unwrap().clone(),
        elapsed: t.start.lock().unwrap().elapsed(),
    }
}

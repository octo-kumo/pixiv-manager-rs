//! Report writer (feature = `profiling`).
//!
//! Drains the tracer + memory counters + optional pprof capture into
//! `report.md` (human) and `report.json` (machine-readable), answering the
//! seven profiling questions:
//!
//! 1. Network — repeated requests? duplicate URLs? latency?
//! 2. File I/O — extra reads/writes? repeated identical writes?
//! 3. Memory — which components allocate the most?
//! 4. Processing — which components consume CPU (pprof + per-message)?
//! 5. Frametime — which message/view hurts the frame budget the most?
//! 6. Frame spikes — frames over 16.7/33.3/100 ms and their culprit message.
//! 7. Storage — total disk written, per-path, biggest single write.

use super::memory::{fmt_bytes, MemoryReport};
use super::{FileOp, Snapshot};
use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

/// Aggregated statistics over a set of durations.
#[derive(Debug, Clone, Copy, Default)]
pub struct Stats {
    pub count: usize,
    pub total: Duration,
    pub avg: Duration,
    pub p50: Duration,
    pub p95: Duration,
    pub max: Duration,
}

/// Compute statistics over durations.
pub fn stats(durations: &[Duration]) -> Stats {
    let count = durations.len();
    if count == 0 {
        return Stats::default();
    }
    let mut sorted: Vec<Duration> = durations.to_vec();
    sorted.sort_unstable();
    let total: Duration = sorted.iter().sum();
    let idx = |p: f64| {
        ((count as f64 * p).ceil() as usize)
            .saturating_sub(1)
            .min(count - 1)
    };
    Stats {
        count,
        total,
        avg: total / count as u32,
        p50: sorted[idx(0.50)],
        p95: sorted[idx(0.95)],
        max: *sorted.last().unwrap(),
    }
}

fn dur_ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

/// Collapse per-hash image-cache paths into one group so a cache directory
/// doesn't dominate the file table with hundreds of near-identical rows.
fn display_path(path: &str) -> String {
    let p = std::path::Path::new(path);
    if p.parent()
        .and_then(|d| d.file_name())
        .map(|f| f == "cache")
        .unwrap_or(false)
    {
        format!("{}/<hash>", p.parent().unwrap().display())
    } else {
        path.to_string()
    }
}

/// Build the JSON body of the snapshot (raw, machine-readable).
pub fn snapshot_json(snapshot: &Snapshot) -> String {
    use serde_json::json;
    let json = json!({
        "elapsed_ms": snapshot.elapsed.as_secs_f64() * 1000.0,
        "http": snapshot.http.iter().map(|s| json!({
            "method": s.method,
            "url": s.url,
            "status": s.status,
            "duration_ms": dur_ms(s.duration),
            "bytes": s.bytes,
            "caller": format!("{}:{}", s.caller.0, s.caller.1),
        })).collect::<Vec<_>>(),
        "file": snapshot.file.iter().map(|s| json!({
            "op": match s.op { FileOp::Read => "read", FileOp::Write => "write" },
            "path": s.path,
            "bytes": s.bytes,
            "duration_ms": dur_ms(s.duration),
            "caller": format!("{}:{}", s.caller.0, s.caller.1),
        })).collect::<Vec<_>>(),
        "update": snapshot.update.iter().map(|s| json!({
            "name": s.name,
            "duration_ms": dur_ms(s.duration),
        })).collect::<Vec<_>>(),
        "view": snapshot.view.iter().map(|s| json!({
            "name": s.name,
            "duration_ms": dur_ms(s.duration),
        })).collect::<Vec<_>>(),
        "task": snapshot.task.iter().map(|s| json!({
            "name": s.name,
            "duration_ms": dur_ms(s.duration),
        })).collect::<Vec<_>>(),
        "frames_ms": snapshot.frames.iter().map(|d| dur_ms(*d)).collect::<Vec<_>>(),
    });
    json.to_string()
}

/// Aggregate phase samples (update/task) by name into stats.
fn phase_stats(samples: &[super::PhaseSample]) -> Vec<(String, Stats)> {
    let mut by_name: BTreeMap<String, Vec<Duration>> = BTreeMap::new();
    for s in samples {
        by_name.entry(s.name.clone()).or_default().push(s.duration);
    }
    by_name
        .into_iter()
        .map(|(name, durations)| (name, stats(&durations)))
        .collect()
}

fn write_markdown(
    snapshot: &Snapshot,
    mem: &MemoryReport,
    pprof: Option<&pprof::Report>,
) -> String {
    let mut md = String::new();
    md.push_str("# Pixiv Manager — Profiling Report\n\n");
    md.push_str(&format!(
        "Session duration: **{:.2}s**\n\n",
        snapshot.elapsed.as_secs_f64()
    ));

    // Per-call-site write counts, hoisted so both the I/O section and the
    // suggestions section can reference them.
    let mut write_callers: BTreeMap<(String, String), usize> = BTreeMap::new();

    // ── 1. Network ────────────────────────────────────────────────
    md.push_str("## 1. Network requests\n\n");
    let total_req = snapshot.http.len();
    let unique: std::collections::HashSet<&str> =
        snapshot.http.iter().map(|h| h.url.as_str()).collect();
    let bytes_down: usize = snapshot.http.iter().map(|h| h.bytes).sum();
    let lat: Vec<Duration> = snapshot.http.iter().map(|h| h.duration).collect();
    let lat_stats = stats(&lat);
    md.push_str(&format!(
        "Requests: **{}** · unique URLs: **{}** · total downloaded: **{}**\n\n",
        total_req,
        unique.len(),
        fmt_bytes(bytes_down as f64)
    ));
    if total_req > 0 {
        md.push_str(&format!(
            "Latency: avg **{:.1}ms** · p50 **{:.1}ms** · p95 **{:.1}ms** · max **{:.1}ms**\n\n",
            dur_ms(lat_stats.avg),
            dur_ms(lat_stats.p50),
            dur_ms(lat_stats.p95),
            dur_ms(lat_stats.max)
        ));
    }

    // Duplicate URLs.
    let mut url_counts: BTreeMap<&str, (usize, usize, Vec<String>)> = BTreeMap::new();
    for h in &snapshot.http {
        let e = url_counts.entry(&h.url).or_insert((0, 0, Vec::new()));
        e.0 += 1;
        e.1 += h.bytes;
        let caller = format!("{}:{}", h.caller.0, h.caller.1);
        if !e.2.contains(&caller) {
            e.2.push(caller);
        }
    }
    let dups: Vec<_> = url_counts.iter().filter(|(_, (n, _, _))| *n > 1).collect();
    if dups.is_empty() {
        md.push_str("✅ **No repeated requests** — every URL was fetched at most once.\n\n");
    } else {
        md.push_str(
            "⚠️ **Repeated requests detected** — the same URL was fetched more than once:\n\n",
        );
        md.push_str("| URL | fetches | bytes | call sites |\n|---|---|---|---|\n");
        for (url, (n, bytes, callers)) in &dups {
            md.push_str(&format!(
                "| `{}` | {} | {} | {} |\n",
                url,
                n,
                fmt_bytes(*bytes as f64),
                callers.join(", ")
            ));
        }
        md.push('\n');
    }

    // Per-endpoint latency.
    if total_req > 0 {
        md.push_str("Per-endpoint latency (top by total time):\n\n");
        let mut by_path: BTreeMap<String, (usize, Duration)> = BTreeMap::new();
        for h in &snapshot.http {
            let path = h.url.split('?').next().unwrap_or(&h.url).to_string();
            let e = by_path.entry(path).or_insert((0, Duration::ZERO));
            e.0 += 1;
            e.1 += h.duration;
        }
        let mut sorted: Vec<_> = by_path.into_iter().collect();
        sorted.sort_by_key(|a| std::cmp::Reverse(a.1 .1));
        md.push_str("| endpoint | count | total |\n|---|---|---|\n");
        for (path, (n, total)) in sorted.iter().take(12) {
            md.push_str(&format!(
                "| `{}` | {} | {:.1}ms |\n",
                path,
                n,
                dur_ms(*total)
            ));
        }
        md.push('\n');
    }

    // ── 2. File I/O ────────────────────────────────────────────────
    md.push_str("## 2. File I/O\n\n");
    if snapshot.file.is_empty() {
        md.push_str("No file I/O recorded.\n\n");
    } else {
        let writes: usize = snapshot
            .file
            .iter()
            .filter(|f| f.op == FileOp::Write)
            .count();
        let reads: usize = snapshot
            .file
            .iter()
            .filter(|f| f.op == FileOp::Read)
            .count();
        let write_bytes: usize = snapshot
            .file
            .iter()
            .filter(|f| f.op == FileOp::Write)
            .map(|f| f.bytes)
            .sum();
        md.push_str(&format!(
            "Writes: **{}** ({}) · Reads: **{}**\n\n",
            writes,
            fmt_bytes(write_bytes as f64),
            reads
        ));
        let mut path_stats: BTreeMap<String, (usize, usize, usize, Duration)> = BTreeMap::new();
        for f in &snapshot.file {
            let key = display_path(&f.path);
            let e = path_stats.entry(key).or_insert((0, 0, 0, Duration::ZERO));
            e.0 += 1;
            e.1 += f.bytes;
            if f.op == FileOp::Write {
                e.2 += 1;
            }
            e.3 += f.duration;
        }
        md.push_str("| path | ops | bytes | writes | total |\n|---|---|---|---|---|\n");
        for (path, (ops, bytes, writes, total)) in path_stats.iter() {
            md.push_str(&format!(
                "| `{}` | {} | {} | {} | {:.2}ms |\n",
                path,
                ops,
                fmt_bytes(*bytes as f64),
                writes,
                dur_ms(*total)
            ));
        }
        md.push('\n');
        // Repeated identical writes (same path, same caller) hint at waste. Keyed by
        // the REAL path (not the collapsed cache display path) so one-time writes to
        // distinct cache hashes are not misreported as amplification.
        for f in snapshot.file.iter().filter(|f| f.op == FileOp::Write) {
            let caller = format!("{}:{}", f.caller.0, f.caller.1);
            *write_callers.entry((f.path.clone(), caller)).or_insert(0) += 1;
        }
        let repeated: Vec<_> = write_callers.iter().filter(|(_, n)| **n > 1).collect();
        if !repeated.is_empty() {
            md.push_str("⚠️ Repeated writes from the same call site:\n\n");
            for ((path, caller), n) in repeated {
                md.push_str(&format!(
                    "- `{}` written **{}×** from `{}`\n",
                    path, n, caller
                ));
            }
            md.push('\n');
        }
    }

    // ── 3. Memory ─────────────────────────────────────────────────
    md.push_str("## 3. Memory\n\n");
    md.push_str(&format!(
        "Peak live: **{}** · current live: **{}** · total allocated: **{}** · freed: **{}**\n\n",
        fmt_bytes(mem.peak_live_bytes as f64),
        fmt_bytes(mem.live_bytes.max(0) as f64),
        fmt_bytes(mem.total_alloc_bytes as f64),
        fmt_bytes(mem.total_dealloc_bytes as f64)
    ));
    if !mem.tags.is_empty() {
        let total: u64 = mem.tags.iter().map(|t| t.alloc_bytes).sum();
        md.push_str("Allocation churn by component:\n\n");
        md.push_str("| component | allocated | % | allocs |\n|---|---|---|---|\n");
        for t in mem.tags.iter().take(20) {
            let pct = if total > 0 {
                t.alloc_bytes as f64 / total as f64 * 100.0
            } else {
                0.0
            };
            md.push_str(&format!(
                "| `{}` | {} | {:.1}% | {} |\n",
                t.tag,
                fmt_bytes(t.alloc_bytes as f64),
                pct,
                t.alloc_count
            ));
        }
        md.push('\n');
    }

    // ── 4. CPU / Processing ───────────────────────────────────────
    md.push_str("## 4. Processing (CPU)\n\n");
    if let Some(report) = pprof {
        md.push_str(&super::cpu::hotspots_markdown(report, 100));
        md.push('\n');
    } else {
        md.push_str("(no pprof capture in this session — run with CPU sampling enabled)\n\n");
    }
    let update_by_name = phase_stats(&snapshot.update);
    if !update_by_name.is_empty() {
        md.push_str("Per-message `update()` cost (top by total):\n\n");
        let mut sorted = update_by_name.clone();
        sorted.sort_by_key(|a| std::cmp::Reverse(a.1.total));
        md.push_str("| message | count | total | avg | p95 | max |\n|---|---|---|---|---|---|\n");
        for (name, s) in sorted.iter().take(20) {
            md.push_str(&format!(
                "| `{}` | {} | {:.1}ms | {:.2}ms | {:.2}ms | {:.2}ms |\n",
                name,
                s.count,
                dur_ms(s.total),
                dur_ms(s.avg),
                dur_ms(s.p95),
                dur_ms(s.max)
            ));
        }
        md.push('\n');
    }
    let task_by_name = phase_stats(&snapshot.task);
    if !task_by_name.is_empty() {
        md.push_str("Async task cost:\n\n");
        let mut sorted = task_by_name.clone();
        sorted.sort_by_key(|a| std::cmp::Reverse(a.1.total));
        md.push_str("| task | count | total | avg | p95 | max |\n|---|---|---|---|---|---|\n");
        for (name, s) in sorted.iter().take(20) {
            md.push_str(&format!(
                "| `{}` | {} | {:.1}ms | {:.2}ms | {:.2}ms | {:.2}ms |\n",
                name,
                s.count,
                dur_ms(s.total),
                dur_ms(s.avg),
                dur_ms(s.p95),
                dur_ms(s.max)
            ));
        }
        md.push('\n');
    }

    // ── 5 + 6. Frametime and spikes ───────────────────────────────
    md.push_str("## 5. Frametime\n\n");
    let view_durations: Vec<Duration> = snapshot.view.iter().map(|s| s.duration).collect();
    let update_durations: Vec<Duration> = snapshot.update.iter().map(|s| s.duration).collect();
    let view_stats = stats(&view_durations);
    let update_stats = stats(&update_durations);
    if snapshot.update.is_empty() && snapshot.view.is_empty() {
        md.push_str("No frame timing captured.\n\n");
    } else {
        md.push_str("| phase | count | avg | p50 | p95 | max |\n|---|---|---|---|---|---|\n");
        if !snapshot.update.is_empty() {
            md.push_str(&format!(
                "| update | {} | {:.2}ms | {:.2}ms | {:.2}ms | **{:.2}ms** |\n",
                update_stats.count,
                dur_ms(update_stats.avg),
                dur_ms(update_stats.p50),
                dur_ms(update_stats.p95),
                dur_ms(update_stats.max)
            ));
        }
        if !snapshot.view.is_empty() {
            md.push_str(&format!(
                "| view | {} | {:.2}ms | {:.2}ms | {:.2}ms | **{:.2}ms** |\n",
                view_stats.count,
                dur_ms(view_stats.avg),
                dur_ms(view_stats.p50),
                dur_ms(view_stats.p95),
                dur_ms(view_stats.max)
            ));
        }
        md.push('\n');
    }

    md.push_str("## 6. Frame spikes\n\n");
    // A frame is approximated as update+view; spike = either over threshold.
    let spike_ms = super::SPIKE_MS;
    for &thresh in spike_ms {
        let mut culprits: BTreeMap<&str, usize> = BTreeMap::new();
        let mut worst: f64 = 0.0;
        let mut worst_msg = "—";
        for s in &snapshot.update {
            let ms = dur_ms(s.duration);
            if ms > thresh {
                *culprits.entry(s.name.as_str()).or_insert(0) += 1;
            }
            if ms > worst {
                worst = ms;
                worst_msg = s.name.as_str();
            }
        }
        for s in &snapshot.view {
            let ms = dur_ms(s.duration);
            if ms > thresh {
                *culprits.entry("view").or_insert(0) += 1;
            }
        }
        if culprits.is_empty() {
            md.push_str(&format!("- Frames over **{thresh}ms**: none ✅\n"));
        } else {
            let total: usize = culprits.values().sum();
            md.push_str(&format!(
                "- Frames over **{thresh}ms**: **{total}** (worst update {worst:.1}ms from `{worst_msg}`)\n"
            ));
            for (name, n) in culprits {
                md.push_str(&format!("  - `{name}` × {n}\n"));
            }
        }
    }
    md.push('\n');

    // ── 7. Storage ────────────────────────────────────────────────
    md.push_str("## 7. Disk storage\n\n");
    let write_bytes: u64 = snapshot
        .file
        .iter()
        .filter(|f| f.op == FileOp::Write)
        .map(|f| f.bytes as u64)
        .sum();
    md.push_str(&format!(
        "Total written to disk: **{}** across **{}** write operations.\n\n",
        fmt_bytes(write_bytes as f64),
        snapshot
            .file
            .iter()
            .filter(|f| f.op == FileOp::Write)
            .count()
    ));
    if let Some(biggest) = snapshot
        .file
        .iter()
        .filter(|f| f.op == FileOp::Write)
        .max_by_key(|f| f.bytes)
    {
        md.push_str(&format!(
            "Largest single write: **{}** to `{}` (from `{}:{}`)\n\n",
            fmt_bytes(biggest.bytes as f64),
            biggest.path,
            biggest.caller.0,
            biggest.caller.1
        ));
    }

    // ── 8. Suggested optimizations ────────────────────────────────
    md.push_str("## 8. Suggested optimizations\n\n");
    let mut suggestions: Vec<String> = Vec::new();

    if !dups.is_empty() {
        let (url, (n, bytes, _)) = dups[0];
        suggestions.push(format!(
            "**Repeated network**: `{}` was fetched **{}×** ({}). Deduplicate in-flight requests or expand the cache so a URL is fetched once per session.",
            url, n, fmt_bytes(*bytes as f64)
        ));
    }
    for (name, s) in phase_stats(&snapshot.update) {
        if s.count >= 5 && dur_ms(s.avg) > 2.0 {
            suggestions.push(format!(
                "**Slow update**: `{}` averages {:.1}ms (p95 {:.1}ms). This is the top candidate for frame jank — inspect its handler for per-frame allocation or O(n) work.",
                name, dur_ms(s.avg), dur_ms(s.p95)
            ));
        }
    }
    if snapshot.view.len() >= 10 && dur_ms(view_stats.avg) > 2.0 {
        suggestions.push(format!(
            "**Slow view**: `view()` averages {:.1}ms (max {:.1}ms). The element tree is rebuilt every frame — consider memoization, fewer visible tiles, or caching string formatting.",
            dur_ms(view_stats.avg), dur_ms(view_stats.max)
        ));
    }
    if let Some(top) = mem.tags.first() {
        if top.alloc_bytes > 0 {
            suggestions.push(format!(
                "**Allocation hot spot**: `{}` allocated {} in {} allocs — the largest churn source. Reducing per-frame allocations here would lower CPU and memory pressure.",
                top.tag,
                fmt_bytes(top.alloc_bytes as f64),
                top.alloc_count
            ));
        }
    }
    for ((path, caller), n) in write_callers.iter().filter(|(_, n)| **n > 5) {
        suggestions.push(format!(
            "**Write amplification**: `{}` is written **{}×** from `{}`. Batch/coalesce these writes (e.g. save on change, not per event).",
            path, n, caller
        ));
    }
    if snapshot.frames.len() >= 10 {
        let f = stats(&snapshot.frames);
        if dur_ms(f.avg) > 17.0 {
            suggestions.push(format!(
                "**Frame cadence**: average interval {:.1}ms (>16.7ms target) — the UI is not holding 60fps.",
                dur_ms(f.avg)
            ));
        }
    }

    if suggestions.is_empty() {
        md.push_str("No obvious optimization opportunities found — the numbers are healthy.\n");
    } else {
        for (i, s) in suggestions.iter().enumerate() {
            md.push_str(&format!("{}. {}\n", i + 1, s));
        }
    }
    md.push('\n');

    md
}

fn write_json(snapshot: &Snapshot, mem: &MemoryReport) -> String {
    let mut json = String::from("{\n");
    json.push_str(&format!(
        "  \"elapsed_ms\": {:.2},\n",
        snapshot.elapsed.as_secs_f64() * 1000.0
    ));
    // memory
    json.push_str("  \"memory\": {\n");
    json.push_str(&format!(
        "    \"peak_live_bytes\": {},\n",
        mem.peak_live_bytes
    ));
    json.push_str(&format!("    \"live_bytes\": {},\n", mem.live_bytes));
    json.push_str(&format!(
        "    \"total_alloc_bytes\": {},\n",
        mem.total_alloc_bytes
    ));
    json.push_str(&format!(
        "    \"total_dealloc_bytes\": {},\n",
        mem.total_dealloc_bytes
    ));
    json.push_str("    \"components\": [\n");
    for (i, t) in mem.tags.iter().enumerate() {
        json.push_str(&format!(
            "      {{\"tag\": {:?}, \"alloc_bytes\": {}, \"alloc_count\": {}}}{}\n",
            t.tag,
            t.alloc_bytes,
            t.alloc_count,
            if i + 1 < mem.tags.len() { "," } else { "" }
        ));
    }
    json.push_str("    ]\n  },\n");
    // http
    json.push_str("  \"http\": [\n");
    for (i, s) in snapshot.http.iter().enumerate() {
        json.push_str(&format!(
            "    {{\"method\": {:?}, \"url\": {:?}, \"status\": {}, \"duration_ms\": {:.2}, \"bytes\": {}, \"caller\": \"{}:{}\"}}{}\n",
            s.method,
            s.url,
            s.status,
            dur_ms(s.duration),
            s.bytes,
            s.caller.0,
            s.caller.1,
            if i + 1 < snapshot.http.len() { "," } else { "" }
        ));
    }
    json.push_str("  ],\n");
    // file
    json.push_str("  \"file\": [\n");
    for (i, s) in snapshot.file.iter().enumerate() {
        json.push_str(&format!(
            "    {{\"op\": {:?}, \"path\": {:?}, \"bytes\": {}, \"duration_ms\": {:.2}, \"caller\": \"{}:{}\"}}{}\n",
            match s.op { FileOp::Read => "read", FileOp::Write => "write" },
            s.path,
            s.bytes,
            dur_ms(s.duration),
            s.caller.0,
            s.caller.1,
            if i + 1 < snapshot.file.len() { "," } else { "" }
        ));
    }
    json.push_str("  ],\n");
    // update/view/task
    for (key, list) in [
        ("update", &snapshot.update),
        ("view", &snapshot.view),
        ("task", &snapshot.task),
    ] {
        json.push_str(&format!("  {key:?}: [\n"));
        for (i, s) in list.iter().enumerate() {
            json.push_str(&format!(
                "    {{\"name\": {:?}, \"duration_ms\": {:.2}}}{}\n",
                s.name,
                dur_ms(s.duration),
                if i + 1 < list.len() { "," } else { "" }
            ));
        }
        json.push_str("  ],\n");
    }
    // frames
    json.push_str("  \"frames_ms\": [\n");
    for (i, d) in snapshot.frames.iter().enumerate() {
        json.push_str(&format!(
            "    {:.2}{}\n",
            dur_ms(*d),
            if i + 1 < snapshot.frames.len() {
                ","
            } else {
                ""
            }
        ));
    }
    json.push_str("  ]\n}\n");
    json
}

/// Write `report.md` + `report.json` into `dir`. Returns `(markdown, json)`.
pub fn write(snapshot: &Snapshot, mem: &MemoryReport, dir: &Path) -> (String, String) {
    let md = write_markdown(snapshot, mem, None);
    let json = write_json(snapshot, mem);
    std::fs::create_dir_all(dir).ok();
    let _ = std::fs::write(dir.join("report.md"), &md);
    let _ = std::fs::write(dir.join("report.json"), &json);
    (md, json)
}

/// Write the report including a pprof CPU capture (flamegraph + hotspots).
pub fn write_with_cpu(
    snapshot: &Snapshot,
    mem: &MemoryReport,
    report: &pprof::Report,
    dir: &Path,
) -> (String, String) {
    let md = write_markdown(snapshot, mem, Some(report));
    // The CPU section is already embedded via write_markdown(Some(report));
    let json = write_json(snapshot, mem);
    std::fs::create_dir_all(dir).ok();
    let mut svg = Vec::new();
    let _ = super::cpu::write_flamegraph(report, &mut svg);
    let _ = std::fs::write(dir.join("flamegraph.svg"), &svg);
    let hotspots = super::cpu::hotspots_markdown(report, 100);
    let _ = std::fs::write(dir.join("hotspots.md"), &hotspots);
    let _ = std::fs::write(dir.join("report.md"), &md);
    let _ = std::fs::write(dir.join("report.json"), &json);
    (md, json)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profiling::tracing;

    fn sample_snapshot() -> Snapshot {
        tracing::clear();
        tracing::record_http(
            "GET",
            "https://x.test/a".into(),
            200,
            Duration::from_millis(10),
            1000,
        );
        tracing::record_http(
            "GET",
            "https://x.test/a".into(),
            200,
            Duration::from_millis(20),
            1000,
        );
        tracing::record_http(
            "GET",
            "https://x.test/b".into(),
            200,
            Duration::from_millis(5),
            100,
        );
        tracing::record_file(
            FileOp::Write,
            "/tmp/lib.json".into(),
            500,
            Duration::from_millis(2),
        );
        tracing::record_file(
            FileOp::Write,
            "/tmp/lib.json".into(),
            500,
            Duration::from_millis(1),
        );
        tracing::record_update("ThumbnailLoaded".into(), Duration::from_millis(30));
        tracing::record_update("GalleryItemClicked".into(), Duration::from_millis(80));
        tracing::record_view("view".into(), Duration::from_millis(12));
        tracing::record_frame(Duration::from_millis(16));
        tracing::record_frame(Duration::from_millis(40));
        tracing::snapshot()
    }

    #[test]
    fn stats_computes_percentiles() {
        let d: Vec<Duration> = (1..=100).map(|i| Duration::from_micros(i * 1000)).collect();
        let s = stats(&d);
        assert_eq!(s.count, 100);
        assert!(s.max >= s.p95 && s.p95 >= s.p50 && s.p50 >= s.avg.min(s.p50));
        assert_eq!(s.p50, Duration::from_millis(50));
    }

    #[test]
    fn report_detects_duplicates_and_slow_updates() {
        let snap = sample_snapshot();
        let mem = super::super::memory::MemoryReport {
            tags: vec![],
            live_bytes: 0,
            peak_live_bytes: 0,
            total_alloc_bytes: 0,
            total_dealloc_bytes: 0,
        };
        let dir = std::env::temp_dir().join(format!("profiler-report-test-{}", std::process::id()));
        let (md, json) = write(&snap, &mem, &dir);
        assert!(md.contains("Repeated requests"), "dup detection");
        assert!(md.contains("GalleryItemClicked"), "slow message listed");
        assert!(md.contains("Repeated writes"), "repeated writes listed");
        assert!(json.contains("\"http\""));
        assert!(json.contains("\"frames_ms\""));
    }
}

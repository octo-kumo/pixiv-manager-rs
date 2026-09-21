//! Tagged `#[global_allocator]` (feature = `profiling`).
//!
//! Wraps `std::alloc::System` and attributes **allocation churn** to the
//! component whose tag was active on the allocating thread (tags are set by
//! [`set_tag`] at `update()`/`view()`/task boundaries in
//! [`super::timed_update`], [`super::timed_view`], [`super::timed_task`]).
//!
//! * Hot path is lock-free: a thread-local slot id + one atomic add per alloc.
//! * Cumulative per-tag bytes/count answer "which component allocates the
//!   most" (allocation churn is the actionable signal in a GC-less Rust GUI).
//! * Global live/peak bytes answer "how much memory is the app holding".
//!
//! The allocator is active only when the `profiling` feature is on (see the
//! `#[global_allocator]` static in `lib.rs`).

use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::Mutex;

const TAG_COUNT: usize = 256;

static ALLOC_BYTES: [AtomicU64; TAG_COUNT] = [const { AtomicU64::new(0) }; TAG_COUNT];
static ALLOC_COUNT: [AtomicU64; TAG_COUNT] = [const { AtomicU64::new(0) }; TAG_COUNT];
static LIVE_BYTES: AtomicI64 = AtomicI64::new(0);
static PEAK_LIVE_BYTES: AtomicU64 = AtomicU64::new(0);
static TOTAL_ALLOC: AtomicU64 = AtomicU64::new(0);
static TOTAL_DEALLOC: AtomicU64 = AtomicU64::new(0);

/// Tag name → slot id. Only consulted when a tag is first seen (thread-local
/// cache makes the steady-state hot path lock-free).
static REGISTRY: Mutex<Option<HashMap<&'static str, usize>>> = Mutex::new(None);

thread_local! {
    static TAG_SLOT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn resolve(tag: &'static str) -> usize {
    let mut registry = REGISTRY.lock().unwrap();
    if registry.is_none() {
        *registry = Some(HashMap::new());
    }
    let registry = registry.as_mut().unwrap();
    if let Some(&slot) = registry.get(tag) {
        return slot;
    }
    if registry.len() < TAG_COUNT {
        let slot = registry.len();
        registry.insert(tag, slot);
        slot
    } else {
        // Registry full: hash to a slot without storing (rare).
        let mut h = 0usize;
        for b in tag.bytes() {
            h = h.wrapping_mul(31).wrapping_add(b as usize);
        }
        h % TAG_COUNT
    }
}

#[inline]
fn current_slot() -> usize {
    TAG_SLOT.with(|c| c.get())
}

/// Set the allocation tag for the current thread (the component that is
/// currently allocating).
pub fn set_tag(tag: &'static str) {
    let slot = resolve(tag);
    TAG_SLOT.with(|c| c.set(slot));
}

/// The global allocator. Attribute every allocation to the current tag.
pub struct TaggedAllocator;

unsafe impl GlobalAlloc for TaggedAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = System.alloc(layout);
        if !ptr.is_null() {
            let size = layout.size() as u64;
            let slot = current_slot();
            ALLOC_BYTES[slot].fetch_add(size, Ordering::Relaxed);
            ALLOC_COUNT[slot].fetch_add(1, Ordering::Relaxed);
            TOTAL_ALLOC.fetch_add(size, Ordering::Relaxed);
            let live = LIVE_BYTES.fetch_add(size as i64, Ordering::Relaxed) + size as i64;
            PEAK_LIVE_BYTES.fetch_max(live as u64, Ordering::Relaxed);
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE_BYTES.fetch_sub(layout.size() as i64, Ordering::Relaxed);
        TOTAL_DEALLOC.fetch_add(layout.size() as u64, Ordering::Relaxed);
        System.dealloc(ptr, layout);
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let ptr = System.realloc(ptr, layout, new_size);
        if !ptr.is_null() {
            let delta = new_size as i64 - layout.size() as i64;
            let slot = current_slot();
            if delta > 0 {
                ALLOC_BYTES[slot].fetch_add(delta as u64, Ordering::Relaxed);
                TOTAL_ALLOC.fetch_add(delta as u64, Ordering::Relaxed);
            }
            let live = LIVE_BYTES.fetch_add(delta, Ordering::Relaxed) + delta;
            PEAK_LIVE_BYTES.fetch_max(live as u64, Ordering::Relaxed);
        }
        ptr
    }
}

/// One component's cumulative allocation counters.
#[derive(Debug, Clone)]
pub struct TagCounters {
    pub tag: &'static str,
    pub alloc_bytes: u64,
    pub alloc_count: u64,
}

/// Memory snapshot: per-tag churn plus global live/peak bytes.
#[derive(Debug, Clone)]
pub struct MemoryReport {
    pub tags: Vec<TagCounters>,
    pub live_bytes: i64,
    pub peak_live_bytes: u64,
    pub total_alloc_bytes: u64,
    pub total_dealloc_bytes: u64,
}

/// Collect the current counters.
pub fn snapshot() -> MemoryReport {
    let registry = REGISTRY.lock().unwrap();
    let mut tags: Vec<TagCounters> = Vec::new();
    if let Some(registry) = registry.as_ref() {
        for (tag, &slot) in registry {
            tags.push(TagCounters {
                tag,
                alloc_bytes: ALLOC_BYTES[slot].load(Ordering::Relaxed),
                alloc_count: ALLOC_COUNT[slot].load(Ordering::Relaxed),
            });
        }
    }
    // Slot 0 with no registered name is the pre-init tag.
    if tags.iter().all(|t| t.alloc_count == 0) {
        // no-op; nothing allocated yet
    }
    tags.sort_by_key(|a| std::cmp::Reverse(a.alloc_bytes));
    MemoryReport {
        tags,
        live_bytes: LIVE_BYTES.load(Ordering::Relaxed),
        peak_live_bytes: PEAK_LIVE_BYTES.load(Ordering::Relaxed),
        total_alloc_bytes: TOTAL_ALLOC.load(Ordering::Relaxed),
        total_dealloc_bytes: TOTAL_DEALLOC.load(Ordering::Relaxed),
    }
}

/// Zero all counters (start of a profiling session).
pub fn reset() {
    for slot in 0..TAG_COUNT {
        ALLOC_BYTES[slot].store(0, Ordering::Relaxed);
        ALLOC_COUNT[slot].store(0, Ordering::Relaxed);
    }
    LIVE_BYTES.store(0, Ordering::Relaxed);
    PEAK_LIVE_BYTES.store(0, Ordering::Relaxed);
    TOTAL_ALLOC.store(0, Ordering::Relaxed);
    TOTAL_DEALLOC.store(0, Ordering::Relaxed);
    *REGISTRY.lock().unwrap() = None;
}

/// Human-readable byte formatting.
pub fn fmt_bytes(bytes: f64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{:.2} {}", value, UNITS[unit])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_assigns_stable_slots() {
        reset();
        let a = resolve("alpha");
        let b = resolve("beta");
        assert_eq!(a, resolve("alpha"));
        assert_eq!(b, resolve("beta"));
        assert_ne!(a, b);
    }

    #[test]
    fn set_tag_switches_current_slot() {
        reset();
        set_tag("one");
        let s1 = current_slot();
        set_tag("two");
        let s2 = current_slot();
        assert_ne!(s1, s2);
    }

    #[test]
    fn snapshot_reports_counters() {
        reset();
        // Allocate under a tag, then switch tags so the report's own
        // allocations don't pollute the measured tag. The global live-bytes
        // counter is shared across parallel tests, so only the per-tag slot
        // (dedicated to this tag) is asserted strictly.
        {
            set_tag("test:alloc");
            let v = vec![0u8; 1024];
            std::hint::black_box(&v);
        }
        set_tag("test:snapshot");
        let rep = snapshot();
        let tc = rep.tags.iter().find(|t| t.tag == "test:alloc").unwrap();
        assert!(tc.alloc_bytes >= 1024, "alloc bytes: {}", tc.alloc_bytes);
        assert!(tc.alloc_count >= 1);
        assert!(rep.total_alloc_bytes >= 1024);
        assert!(rep.peak_live_bytes >= 1024);
    }
}

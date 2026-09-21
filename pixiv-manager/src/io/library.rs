//! Persistent local bookmark library + download job queue.
//!
//! Both are stored as JSON files and written **atomically** (temp file +
//! rename), so a crash mid-write can never corrupt the committed data. This
//! makes the bookmark sync and the download-all job safe to interrupt and
//! resume across app restarts.

use chrono::{DateTime, Utc};
use pixivj_rs::model::Illustration;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

// ── Bookmark library ────────────────────────────────────────────────────

/// The full locally-mirrored bookmark list.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BookmarkLibrary {
    pub illusts: Vec<Illustration>,
    /// When the library was last fully re-synced (used for periodic rebuilds).
    #[serde(default)]
    pub last_full_sync: Option<DateTime<Utc>>,
    /// The deepest pagination point reached by the last sync (the `next_url`
    /// after the last page actually merged). Lets a relaunch continue pulling
    /// old bookmarks past an interrupted run, while new bookmarks still stop
    /// early at the first page we already know. `None` = fully synced.
    #[serde(default)]
    pub cursor: Option<String>,
    /// The deepest Twitter likes cursor reached by the last sync (same role
    /// as `cursor` for the Twitter half of the library). `None` = done.
    #[serde(default)]
    pub twitter_cursor: Option<String>,
    /// O(1) membership index of every illust id, kept in sync with `illusts`
    /// by `merge`/`full_rebuild`/`load`. Not persisted (rebuilt on load).
    #[serde(skip, default)]
    ids: std::collections::HashSet<u64>,
}

impl BookmarkLibrary {
    /// Load from disk (missing or corrupt files → empty library).
    pub fn load(path: &Path) -> Self {
        match crate::io::read_file(path) {
            Some(content) => {
                let lib: Self = serde_json::from_str(&content).unwrap_or_default();
                lib.with_ids()
            }
            None => Self::default(),
        }
    }

    /// Rebuild the id index from `illusts` (after deserialization).
    fn with_ids(mut self) -> Self {
        self.ids = self.illusts.iter().map(|i| i.id).collect();
        self
    }

    /// Write atomically: temp file + rename. A crash can only leave a `.tmp`
    /// file behind, never a corrupt library at the final path.
    pub fn save_atomic(&self, path: &Path) -> std::io::Result<()> {
        crate::io::save_json_atomic(self, path)
    }

    /// Merge new illusts, deduplicated by id (newest-first order preserved).
    /// Returns the illusts that were actually new.
    pub fn merge(&mut self, new: Vec<Illustration>) -> Vec<Illustration> {
        let mut added = Vec::new();
        for illust in new {
            if self.ids.insert(illust.id) {
                self.illusts.push(illust.clone());
                added.push(illust);
            }
        }
        added
    }

    /// Whether every illust in `new` already exists (used to stop incremental
    /// syncs once we reach data we already know).
    pub fn all_known(&self, new: &[Illustration]) -> bool {
        !new.is_empty() && new.iter().all(|i| self.ids.contains(&i.id))
    }

    /// Whether an item is in the library.
    pub fn contains(&self, id: u64) -> bool {
        self.ids.contains(&id)
    }

    /// Remove an item (used by locally-managed providers such as Bilibili,
    /// where un-bookmarking must actually delete the entry). Returns whether
    /// it was present.
    pub fn remove(&mut self, id: u64) -> bool {
        if !self.ids.remove(&id) {
            return false;
        }
        self.illusts.retain(|i| i.id != id);
        true
    }

    /// Replace the whole library (full re-sync; drops removed bookmarks).
    pub fn full_rebuild(&mut self, all: Vec<Illustration>) {
        self.ids = all.iter().map(|i| i.id).collect();
        self.illusts = all;
        self.last_full_sync = Some(Utc::now());
        self.cursor = None;
        self.twitter_cursor = None;
    }
}

// ── Download job queue ──────────────────────────────────────────────────

/// A persistent queue of illust ids to download (for "download all").
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DownloadQueue {
    /// Illust ids that have been enqueued but not yet completed.
    pub remaining: Vec<u64>,
    /// Total ids ever enqueued in this job (done = total - remaining.len()).
    /// Persisted so progress survives restarts.
    pub total: usize,
    /// Illust ids that permanently failed (deleted from Pixiv — HTTP 404/410),
    /// so a "download all" doesn't re-attempt them every run.
    #[serde(default)]
    pub failed: Vec<u64>,
}

impl DownloadQueue {
    /// Load from disk; missing/corrupt files → empty queue.
    pub fn load(path: &Path) -> Option<Self> {
        serde_json::from_str(&crate::io::read_file(path)?).ok()
    }

    /// Write atomically.
    pub fn save_atomic(&self, path: &Path) -> std::io::Result<()> {
        crate::io::save_json_atomic(self, path)
    }

    /// Add an id to the queue (returns false if it's already queued or is a
    /// known-permanently-failed id).
    pub fn enqueue(&mut self, id: u64) -> bool {
        if self.remaining.contains(&id) || self.failed.contains(&id) {
            return false;
        }
        self.remaining.push(id);
        self.total += 1;
        true
    }

    /// Mark an id as completed (removes it from the queue).
    pub fn mark_done(&mut self, id: u64) {
        self.remaining.retain(|&r| r != id);
    }

    /// Whether an id permanently failed (deleted) and should not be retried.
    pub fn is_failed(&self, id: u64) -> bool {
        self.failed.contains(&id)
    }

    /// Record a permanent failure (deleted illust) and drop it from the
    /// queue so future runs skip it.
    pub fn mark_failed(&mut self, id: u64) {
        self.remaining.retain(|&r| r != id);
        if !self.failed.contains(&id) {
            self.failed.push(id);
        }
    }

    /// Raise an item's priority: move it one slot earlier in the queue
    /// (earlier = downloaded sooner). Returns whether it moved.
    pub fn move_up(&mut self, id: u64) -> bool {
        let Some(pos) = self.remaining.iter().position(|&r| r == id) else {
            return false;
        };
        if pos == 0 {
            return false;
        }
        self.remaining.swap(pos, pos - 1);
        true
    }

    /// Lower an item's priority: move it one slot later in the queue.
    /// Returns whether it moved.
    pub fn move_down(&mut self, id: u64) -> bool {
        let Some(pos) = self.remaining.iter().position(|&r| r == id) else {
            return false;
        };
        if pos + 1 >= self.remaining.len() {
            return false;
        }
        self.remaining.swap(pos, pos + 1);
        true
    }

    /// Completed count.
    pub fn done(&self) -> usize {
        self.total.saturating_sub(self.remaining.len())
    }

    /// Whether every enqueued item has completed.
    pub fn is_finished(&self) -> bool {
        self.remaining.is_empty()
    }

    /// Progress fraction [0, 1].
    pub fn fraction(&self) -> f32 {
        if self.total == 0 {
            0.0
        } else {
            self.done() as f32 / self.total as f32
        }
    }
}

// ── Job status (UI-facing) ──────────────────────────────────────────────

/// Status of the bookmark library sync job.
#[derive(Debug, Clone, Default)]
pub struct SyncJobState {
    pub running: bool,
    /// Pages processed.
    pub done: usize,
    /// Total pages, when known (pagination makes this unknown until the end).
    pub total: Option<usize>,
    pub current: String,
    pub error: Option<String>,
}

/// Status of the download job (the shared, persistent queue).
#[derive(Debug, Clone, Default)]
pub struct DownloadJobState {
    pub running: bool,
    pub current: String,
    pub error: Option<String>,
    /// Illust ids currently downloading (in-memory; not persisted).
    pub in_flight: Vec<u64>,
}

// ── Shared paths ────────────────────────────────────────────────────────

/// Directory holding the local library and job state.
pub fn library_dir() -> PathBuf {
    // Tests set this to a temp dir so they never touch real user data.
    if let Ok(dir) = std::env::var("PIXIV_MANAGER_CONFIG_DIR") {
        if !dir.is_empty() {
            return PathBuf::from(dir).join("library");
        }
    }
    // Test builds (feature `test-util`) never touch the real library either;
    // respect XDG_DATA_HOME redirects used by the CLI tests, else a temp dir.
    #[cfg(feature = "test-util")]
    if std::env::var("XDG_DATA_HOME")
        .map(|d| !d.is_empty())
        .unwrap_or(false)
    {
        // fall through to dirs::data_dir()
    } else {
        return std::env::temp_dir()
            .join(format!("pixiv-manager-test-{}", std::process::id()))
            .join("library");
    }
    dirs::data_dir()
        .map(|d| d.join("pixiv-manager").join("library"))
        .unwrap_or_else(|| std::env::temp_dir().join("pixiv-manager-library"))
}

pub fn bookmarks_library_path() -> PathBuf {
    library_dir().join("bookmarks.json")
}

pub fn download_queue_path() -> PathBuf {
    library_dir().join("download_queue.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use pixivj_rs::model::{Author, Illustration};

    fn illust(id: u64) -> Illustration {
        Illustration {
            id,
            title: Some(format!("t{}", id)),
            illust_type: None,
            image_urls: None,
            caption: None,
            restrict: None,
            user: Some(Author {
                id: 1,
                profile_image_urls: None,
                name: "a".into(),
                account: None,
                comment: None,
                is_followed: None,
            }),
            tags: vec![],
            tools: vec![],
            create_date: None,
            page_count: Some(1),
            width: None,
            height: None,
            sanity_level: None,
            x_restrict: None,
            series: None,
            meta_single_page: None,
            meta_pages: vec![],
            total_view: None,
            total_bookmarks: None,
            is_bookmarked: None,
            visible: None,
            is_muted: None,
            illust_ai_type: None,
            provider_data: None,
        }
    }

    #[test]
    fn library_merge_dedup() {
        let mut lib = BookmarkLibrary::default();
        assert_eq!(lib.merge(vec![illust(1), illust(2)]).len(), 2);
        assert_eq!(lib.merge(vec![illust(2), illust(3)]).len(), 1);
        assert_eq!(lib.illusts.len(), 3);
        assert!(lib.illusts.iter().any(|i| i.id == 1));
        assert!(lib.illusts.iter().any(|i| i.id == 3));
    }

    #[test]
    fn library_all_known() {
        let mut lib = BookmarkLibrary::default();
        lib.merge(vec![illust(1), illust(2)]);
        assert!(lib.all_known(&[illust(1), illust(2)]));
        assert!(!lib.all_known(&[illust(1), illust(9)]));
        assert!(!lib.all_known(&[]));
    }

    #[test]
    fn library_save_load_roundtrip_atomic() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("bookmarks.json");
        let mut lib = BookmarkLibrary::default();
        lib.merge(vec![illust(42)]);
        lib.full_rebuild(vec![illust(7), illust(8)]);
        lib.save_atomic(&path).unwrap();

        let loaded = BookmarkLibrary::load(&path);
        assert_eq!(loaded.illusts.len(), 2);
        assert_eq!(loaded.illusts[0].id, 7);
        assert!(loaded.last_full_sync.is_some());
        // No stray temp files.
        assert!(!tmp.path().join("bookmarks.tmp").exists());
    }

    #[test]
    fn library_load_missing_or_corrupt_is_empty() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(BookmarkLibrary::load(&tmp.path().join("nope.json"))
            .illusts
            .is_empty());
        let bad = tmp.path().join("bad.json");
        std::fs::write(&bad, b"not json").unwrap();
        assert!(BookmarkLibrary::load(&bad).illusts.is_empty());
    }

    #[test]
    fn queue_enqueue_mark_done() {
        let mut q = DownloadQueue::default();
        assert!(q.enqueue(1));
        assert!(q.enqueue(2));
        assert!(!q.enqueue(1), "dedup");
        assert_eq!(q.total, 2);
        assert_eq!(q.done(), 0);
        q.mark_done(1);
        assert_eq!(q.done(), 1);
        assert_eq!(q.fraction(), 0.5);
        assert!(!q.remaining.contains(&1));
        assert!(!q.is_finished());
        q.mark_done(2);
        assert!(q.is_finished());
        assert_eq!(q.fraction(), 1.0);
    }

    #[test]
    fn queue_save_load_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("queue.json");
        let mut q = DownloadQueue::default();
        q.enqueue(5);
        q.enqueue(6);
        q.mark_done(5);
        q.save_atomic(&path).unwrap();
        let loaded = DownloadQueue::load(&path).unwrap();
        assert_eq!(loaded.remaining, vec![6]);
        assert_eq!(loaded.total, 2);
        assert_eq!(loaded.done(), 1);
    }

    #[test]
    fn queue_missing_load_is_none() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(DownloadQueue::load(&tmp.path().join("none.json")).is_none());
    }

    #[test]
    fn queue_mark_failed_skips_future_enqueues() {
        let mut q = DownloadQueue::default();
        q.enqueue(1);
        q.enqueue(2);
        q.mark_failed(1); // e.g. deleted illust → permanent
        assert!(q.is_failed(1));
        assert!(!q.remaining.contains(&1), "failed id leaves the queue");
        // A later "download all" must not re-enqueue a failed id.
        assert!(!q.enqueue(1), "failed id is never re-enqueued");
        assert!(q.enqueue(3), "other ids still enqueue");
        assert_eq!(q.total, 3, "failed id still counts toward progress");
    }

    #[test]
    fn queue_move_up_down_reorders_priority() {
        let mut q = DownloadQueue::default();
        q.enqueue(1);
        q.enqueue(2);
        q.enqueue(3);
        // Move 3 up one slot.
        assert!(q.move_up(3));
        assert_eq!(q.remaining, vec![1, 3, 2]);
        // Move it up again → front.
        assert!(q.move_up(3));
        assert_eq!(q.remaining, vec![3, 1, 2]);
        // It can't move past the front.
        assert!(!q.move_up(3));
        assert_eq!(q.remaining, vec![3, 1, 2]);
        // Move 1 down.
        assert!(q.move_down(1));
        assert_eq!(q.remaining, vec![3, 2, 1]);
        // Last item can't move down.
        assert!(!q.move_down(1));
        // Unknown ids do nothing.
        assert!(!q.move_up(99));
        assert!(!q.move_down(99));
    }

    #[test]
    fn queue_failed_persists_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("queue.json");
        let mut q = DownloadQueue::default();
        q.enqueue(1);
        q.enqueue(2);
        q.mark_failed(2);
        q.save_atomic(&path).unwrap();
        let loaded = DownloadQueue::load(&path).unwrap();
        assert!(loaded.is_failed(2));
        assert!(!loaded.remaining.contains(&2));
        // Old queue files without the `failed` field load as empty.
        std::fs::write(&path, r#"{"remaining":[5],"total":1}"#).unwrap();
        let legacy = DownloadQueue::load(&path).unwrap();
        assert!(legacy.failed.is_empty());
        assert_eq!(legacy.remaining, vec![5]);
    }
}

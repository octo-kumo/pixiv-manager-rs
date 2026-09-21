use std::collections::{HashMap, HashSet};

/// The per-illust download state the UI shows as a progress bar.
#[derive(Debug, Clone, Copy, Default)]
pub struct DownloadState {
    /// Overall completion fraction [0.0, 1.0] reported by the downloader
    /// (across all pages).
    pub fraction: f32,
}

impl DownloadState {
    /// The overall fraction used for UI progress.
    pub fn progress_fraction(&self) -> f32 {
        self.fraction
    }
}

/// Tracks all active downloads, preventing duplicates, and remembers
/// successfully completed downloads so the UI can keep a button disabled.
#[derive(Debug, Clone, Default)]
pub struct DownloadTracker {
    /// Map of illust_id → download state.
    downloads: HashMap<u64, DownloadState>,
    /// Illust IDs whose download completed successfully.
    completed: HashSet<u64>,
}

impl DownloadTracker {
    /// Check if a download is already in progress for this illust.
    pub fn is_downloading(&self, illust_id: u64) -> bool {
        self.downloads.contains_key(&illust_id)
    }

    /// Whether a download for this illust finished successfully.
    pub fn is_completed(&self, illust_id: u64) -> bool {
        self.completed.contains(&illust_id)
    }

    /// Register a new download. Returns false if already downloading.
    pub fn start(&mut self, illust_id: u64) -> bool {
        if self.downloads.contains_key(&illust_id) {
            return false; // dedup
        }
        self.downloads.insert(illust_id, DownloadState::default());
        true
    }

    /// Set the overall completion fraction for an active download.
    pub fn set_fraction(&mut self, illust_id: u64, fraction: f32) {
        if let Some(state) = self.downloads.get_mut(&illust_id) {
            state.fraction = fraction.clamp(0.0, 1.0);
        }
    }

    /// Mark a download as completed and remove it from tracking.
    pub fn complete(&mut self, illust_id: u64) -> Option<DownloadState> {
        self.downloads.remove(&illust_id)
    }

    /// Mark a successfully completed download so the UI stays disabled.
    pub fn mark_completed(&mut self, illust_id: u64) {
        self.completed.insert(illust_id);
    }

    /// Get the state of an active download, if it exists.
    pub fn get(&self, illust_id: u64) -> Option<&DownloadState> {
        self.downloads.get(&illust_id)
    }

    /// The aggregated progress fraction of an in-progress download, or `None`
    /// when nothing is downloading for this illust (queued items only get
    /// their fraction once their worker starts, so the gallery tile's bar
    /// appears exactly while bytes are moving and disappears on completion).
    pub fn progress(&self, illust_id: u64) -> Option<f32> {
        self.downloads
            .get(&illust_id)
            .map(DownloadState::progress_fraction)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_download_tracker_dedup() {
        let mut tracker = DownloadTracker::default();
        assert!(tracker.start(42));
        assert!(!tracker.start(42)); // duplicate
        assert!(tracker.start(43)); // different id
    }

    #[test]
    fn test_download_tracker_lifecycle() {
        let mut tracker = DownloadTracker::default();
        tracker.start(1);
        assert!(tracker.is_downloading(1));

        tracker.set_fraction(1, 0.5);
        assert_eq!(tracker.complete(1).unwrap().fraction, 0.5);
        assert!(!tracker.is_downloading(1));
    }

    #[test]
    fn test_download_tracker_completed_state() {
        let mut tracker = DownloadTracker::default();
        tracker.start(7);
        tracker.set_fraction(7, 0.42);
        tracker.complete(7);
        assert!(!tracker.is_completed(7));

        tracker.start(8);
        tracker.mark_completed(8);
        tracker.complete(8);
        assert!(tracker.is_completed(8));
        assert!(!tracker.is_downloading(8));
    }

    #[test]
    fn test_set_fraction_clamps() {
        let mut tracker = DownloadTracker::default();
        tracker.start(1);
        tracker.set_fraction(1, 2.0);
        assert_eq!(tracker.complete(1).unwrap().fraction, 1.0);
    }

    #[test]
    fn progress_reports_aggregated_fraction_and_clears_on_completion() {
        let mut tracker = DownloadTracker::default();
        // Not downloading → the tile shows no bar.
        assert_eq!(tracker.progress(9), None);
        tracker.start(9);
        // The downloader reports the aggregate across the item's pages.
        tracker.set_fraction(9, 0.25);
        assert_eq!(tracker.progress(9), Some(0.25));
        tracker.set_fraction(9, 0.75);
        assert_eq!(tracker.progress(9), Some(0.75));
        // Completion removes the entry → the bar disappears.
        tracker.complete(9);
        tracker.mark_completed(9);
        assert_eq!(tracker.progress(9), None);
        assert!(tracker.is_completed(9));
    }
}

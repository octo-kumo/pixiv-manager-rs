//! Concurrency stress tests for the atomic persistence layer.
//!
//! The property under test is not "which writer wins" but "the file is always
//! one complete writer's value": never mixed bytes, never truncated JSON, never
//! missing because two replacements raced.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Barrier, Mutex};

use pixiv_manager::app::state::Settings;
use pixiv_manager::io::library::{BookmarkLibrary, DownloadQueue};
use pixiv_manager::io::paths::AppPaths;
use pixiv_manager::test_util::mock_illust;

const THREADS: usize = 4;
const WRITES_PER_THREAD: usize = 25;

/// Run `write` from several threads at once and return every value written.
fn hammer<W>(write: W) -> Vec<u32>
where
    W: Fn(u32) + Send + Sync,
{
    let barrier = Barrier::new(THREADS);
    std::thread::scope(|scope| {
        for thread in 0..THREADS {
            let barrier = &barrier;
            let write = &write;
            scope.spawn(move || {
                barrier.wait();
                for i in 0..WRITES_PER_THREAD {
                    write((thread * WRITES_PER_THREAD + i) as u32);
                }
            });
        }
    });
    (0..(THREADS * WRITES_PER_THREAD) as u32).collect()
}

fn no_temp_files_left(dir: &Path) {
    let leftovers: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| pixiv_manager::io::is_atomic_temp_file(name.as_ref()))
        .collect();
    assert!(
        leftovers.is_empty(),
        "temp files left behind: {leftovers:?}"
    );
}

/// Serializes the stores that resolve their path through `AppPaths`, which is
/// process-global in test builds.
static PATHS_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn concurrent_writes_always_leave_one_complete_value() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.json");

    let written = hammer(|n| {
        let payload = format!(r#"{{"writer":{n},"padding":"{}"}}"#, "x".repeat(2048));
        pixiv_manager::io::write_atomic(&path, payload.as_bytes()).unwrap();
    });

    let content = std::fs::read_to_string(&path).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&content).expect("complete JSON");
    let winner = parsed["writer"].as_u64().expect("writer field") as u32;
    assert!(written.contains(&winner), "unknown writer value {winner}");
    assert_eq!(parsed["padding"].as_str().unwrap().len(), 2048);
    no_temp_files_left(dir.path());
}

#[test]
fn concurrent_bookmark_library_saves_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let path: PathBuf = dir.path().join("bookmarks.json");

    hammer(|n| {
        let mut library = BookmarkLibrary::default();
        library.merge(vec![mock_illust(u64::from(n) + 1)]);
        library.save_atomic(&path).unwrap();
    });

    let loaded = BookmarkLibrary::load(&path);
    assert_eq!(loaded.illusts.len(), 1, "exactly one complete library");
    no_temp_files_left(dir.path());
}

#[test]
fn concurrent_download_queue_saves_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("download_queue.json");

    hammer(|n| {
        let mut queue = DownloadQueue::default();
        queue.enqueue(u64::from(n) + 1);
        queue.enqueue(u64::from(n) + 1000);
        queue.save_atomic(&path).unwrap();
    });

    let loaded = DownloadQueue::load(&path).expect("queue readable");
    assert_eq!(loaded.remaining.len(), 2);
    no_temp_files_left(dir.path());
}

#[test]
fn concurrent_color_palette_saves_roundtrip() {
    let _guard = PATHS_LOCK.lock().unwrap();
    let paths = AppPaths::discover();
    let _ = std::fs::remove_file(paths.colors_file());

    hammer(|n| {
        let mut palettes = HashMap::new();
        palettes.insert(u64::from(n) + 1, vec![[n as u8, 0, 0]]);
        pixiv_manager::io::colors::save_colors(&palettes);
    });

    let loaded = pixiv_manager::io::colors::load_colors();
    assert_eq!(loaded.len(), 1, "one complete palette map survived");
    no_temp_files_left(&paths.library_dir());
}

#[test]
fn concurrent_viewer_transform_saves_roundtrip() {
    let _guard = PATHS_LOCK.lock().unwrap();
    let paths = AppPaths::discover();
    let _ = std::fs::remove_file(paths.viewer_transforms_file());

    hammer(|n| {
        let mut transform = pixiv_manager::viewer_tools::ViewTransform::default();
        transform.rotate(0, (n % 3) as i8 + 1);
        let mut map = HashMap::new();
        map.insert(u64::from(n) + 1, transform);
        pixiv_manager::viewer_tools::save(&map);
    });

    let loaded = pixiv_manager::viewer_tools::load();
    assert_eq!(loaded.len(), 1, "one complete transform map survived");
    no_temp_files_left(&paths.library_dir());
}

#[test]
fn concurrent_metadata_entries_stay_parseable() {
    let _guard = PATHS_LOCK.lock().unwrap();
    let paths = AppPaths::discover();
    let _ = std::fs::remove_dir_all(paths.meta_dir());

    hammer(|n| {
        let illust = mock_illust(u64::from(n) % 8 + 1);
        pixiv_manager::io::meta_cache::save_detail(u64::from(n) % 8 + 1, &illust);
    });

    for id in 1..=8 {
        let loaded = pixiv_manager::io::meta_cache::load_detail(id)
            .unwrap_or_else(|| panic!("entry {id} missing"));
        assert_eq!(loaded.id, id);
    }
    no_temp_files_left(&paths.meta_dir());
}

#[test]
fn concurrent_config_saves_keep_the_file_loadable() {
    let _guard = PATHS_LOCK.lock().unwrap();
    let paths = AppPaths::discover();
    let _ = std::fs::remove_file(paths.config_file());

    hammer(|n| {
        let settings = Settings {
            refresh_token: Some(format!("token-{n}")),
            grid_size: 100 + n,
            ..Default::default()
        };
        pixiv_manager::io::config::save_config(&settings);
    });

    let loaded = pixiv_manager::io::config::load_config().expect("config parses");
    let token = loaded.refresh_token.expect("token present");
    assert!(token.starts_with("token-"), "torn token: {token:?}");
    assert!(loaded.grid_size >= 100);
    no_temp_files_left(&paths.config_dir);
}

/// Repeated saves of every store, sequentially: the replacement of an existing
/// file must keep working (it did not on Windows with the old writer).
#[test]
fn repeated_saves_overwrite_and_stay_valid() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bookmarks.json");
    let mut seen = HashSet::new();
    for n in 0..10u64 {
        let mut library = BookmarkLibrary::default();
        library.merge(vec![mock_illust(n + 1)]);
        library.save_atomic(&path).unwrap();
        let loaded = BookmarkLibrary::load(&path);
        assert_eq!(loaded.illusts.len(), 1);
        assert!(seen.insert(loaded.illusts[0].id), "stale content survived");
    }
}

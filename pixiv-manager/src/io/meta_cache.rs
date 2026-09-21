//! Atomic, compact, corrupt-safe on-disk cache for illust metadata (the
//! detail `Illustration`) and comments.
//!
//! Each illust gets its own `meta/{id}.json` file, written atomically
//! (temp file + rename) in compact (non-pretty) form. A crash can only leave
//! a `.tmp` file behind, never a half-written entry at the final path — and a
//! corrupt/missing entry is treated as a miss, so the cache can never break
//! the app.

use pixivj_rs::model::{Comment, Illustration};
use std::path::PathBuf;

/// Where per-illust metadata lives (alongside the library).
fn meta_dir() -> PathBuf {
    crate::io::library::library_dir().join("meta")
}

/// Write `value` as compact JSON to `path` atomically. Returns false on
/// failure (the caller treats a miss as "not cached").
fn write_json_atomic<T: serde::Serialize>(path: &PathBuf, value: &T) -> bool {
    let Ok(content) = serde_json::to_string(value) else {
        return false;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let tmp = path.with_extension("tmp");
    if std::fs::write(&tmp, content).is_err() {
        return false;
    }
    match std::fs::rename(&tmp, path) {
        Ok(()) => true,
        Err(_) if path.exists() => {
            let _ = std::fs::remove_file(path);
            std::fs::rename(&tmp, path).is_ok()
        }
        Err(_) => false,
    }
}

fn read_json_atomic<T: serde::de::DeserializeOwned>(path: &PathBuf) -> Option<T> {
    let content = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

/// The cached detail illustration for `id`.
pub fn load_detail(id: u64) -> Option<Illustration> {
    read_json_atomic(&meta_dir().join(format!("{id}.json")))
}

/// Persist a detail illustration for `id`.
pub fn save_detail(id: u64, illust: &Illustration) {
    write_json_atomic(&meta_dir().join(format!("{id}.json")), illust);
}

/// The cached comments for `id`.
pub fn load_comments(id: u64) -> Option<Vec<Comment>> {
    read_json_atomic(&meta_dir().join(format!("{id}_comments.json")))
}

/// Persist comments for `id`.
pub fn save_comments(id: u64, comments: &[Comment]) {
    write_json_atomic(
        &meta_dir().join(format!("{id}_comments.json")),
        &comments.to_vec(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use pixivj_rs::model::{Author, Illustration};

    fn illust(id: u64) -> Illustration {
        Illustration {
            id,
            title: Some(format!("t{id}")),
            user: Some(Author {
                id: 1,
                name: "a".into(),
                account: None,
                profile_image_urls: None,
                comment: None,
                is_followed: None,
            }),
            tags: vec![],
            tools: vec![],
            create_date: None,
            page_count: Some(2),
            ..Illustration::default_local()
        }
    }

    #[test]
    fn detail_roundtrip_and_corruption_tolerance() {
        crate::test_util::set_test_config_dir();

        assert!(load_detail(7).is_none(), "missing → miss");
        save_detail(7, &illust(7));
        assert_eq!(load_detail(7).unwrap().id, 7);
        assert_eq!(load_detail(7).unwrap().page_count, Some(2));

        // Corrupt the file → treated as a miss (never a panic).
        let path = meta_dir().join("7.json");
        std::fs::write(&path, b"not json").unwrap();
        assert!(load_detail(7).is_none());
    }

    #[test]
    fn comments_roundtrip() {
        crate::test_util::set_test_config_dir();

        let comments = vec![Comment {
            id: 1,
            comment: "hi".into(),
            date: None,
            has_replies: None,
            user: None,
            stamp: None,
        }];
        save_comments(3, &comments);
        let loaded = load_comments(3).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].comment, "hi");
    }
}

//! Shared test fixtures, compiled only for tests/benches via the `test-util`
//! feature (see Cargo.toml). Keeps `mock_illust` from being copy-pasted across
//! the bench suite and the proptest suite.

use pixivj_rs::model::*;

/// Point `PIXIV_MANAGER_CONFIG_DIR` at one shared leaked temp dir, so tests
/// that persist config/library/colors never touch real user data and never
/// race each other (tests run in parallel in the same process). Safe to call
/// from any test; the dir is set exactly once.
#[cfg(test)]
pub fn set_test_config_dir() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let dir = tempfile::tempdir().expect("temp dir");
        std::env::set_var("PIXIV_MANAGER_CONFIG_DIR", dir.path());
        std::mem::forget(dir); // keep the dir alive for the whole test run
    });
}

/// A deterministic mock [`Illustration`] for benchmarks and property tests.
pub fn mock_illust(id: u64) -> Illustration {
    Illustration {
        id,
        title: Some(format!("Illust {}", id)),
        illust_type: None,
        image_urls: Some(IllustImageUrls {
            square_medium: Some("thumb.jpg".into()),
            medium: Some("thumb.jpg".into()),
            large: Some("large.jpg".into()),
        }),
        caption: None,
        restrict: if id % 7 == 0 { Some(1) } else { Some(0) },
        user: Some(Author {
            id: id % 100,
            profile_image_urls: None,
            name: format!("artist_{}", id % 10),
            account: None,
            comment: None,
            is_followed: None,
        }),
        tags: vec![
            Tag {
                name: format!("tag_{}", id % 5),
                translated_name: None,
            },
            Tag {
                name: "cat".into(),
                translated_name: None,
            },
        ],
        tools: if id % 3 == 0 {
            vec!["Photoshop".into()]
        } else {
            vec![]
        },
        create_date: Some(chrono::DateTime::from_timestamp(id as i64 * 86400, 0).unwrap()),
        page_count: Some((id % 10 + 1) as i32),
        width: Some(((id % 3 + 1) * 500) as i32),
        height: Some(((id % 4 + 1) * 500) as i32),
        sanity_level: Some((id % 6) as i32),
        x_restrict: Some((id % 3) as i32),
        series: None,
        meta_single_page: None,
        meta_pages: vec![],
        total_view: Some((id * 100) % 50000),
        total_bookmarks: Some((id * 10) % 5000),
        is_bookmarked: Some(id % 2 == 0),
        visible: Some(true),
        is_muted: None,
        illust_ai_type: Some(if id % 5 == 0 { 2 } else { 0 }),
        provider_data: None,
    }
}

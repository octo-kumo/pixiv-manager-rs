//! Shared, front-end-agnostic business logic used by both the GUI and the CLI
//! so behavior (filename scheme, download URL collection, config, …) can never
//! drift between them.

use pixivj_rs::api::PixivApiClient;
use pixivj_rs::auth::{LazyTokenRefresher, PixivOAuthClient};
use pixivj_rs::error::PixivError;
use pixivj_rs::model::Illustration;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Build a Pixiv API client from a refresh token, applying the configured
/// proxy and registering `on_refresh` so rotated refresh tokens are always
/// persisted (Pixiv rotates them on every refresh — dropping them would
/// silently invalidate the saved login).
///
/// With `eager == true` the client authenticates immediately (fail-fast; used
/// by the CLI so errors surface at startup), otherwise tokens refresh lazily
/// on the first request (used by the GUI, which falls back to onboarding).
pub async fn build_client(
    refresh_token: String,
    proxy: Option<String>,
    on_refresh: impl Fn(String) + Send + Sync + 'static,
    eager: bool,
) -> Result<PixivApiClient, PixivError> {
    let oauth = PixivOAuthClient::new_with_proxy(proxy.clone())?;
    let refresher = Arc::new(LazyTokenRefresher::new(oauth, refresh_token));
    refresher.set_on_refresh(on_refresh);
    if eager {
        refresher.refresh_now().await?;
    }
    PixivApiClient::new_with_proxy(refresher, proxy)
}

/// The default directory for downloaded illusts when no gallery path is set
/// (`~/Pictures/pixiv`, falling back to `./pixiv`).
pub fn default_gallery_dir() -> PathBuf {
    dirs::picture_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("pixiv")
}

/// The per-illust download directory: `{gallery}/{id}` for Pixiv and
/// `{gallery}/tw_{native}` for tweets — no artwork title in the path (the
/// title can contain arbitrary characters and duplicates; the numeric id is
/// the only stable key). Both forms are pure integers (with the `tw_` tag),
/// so path traversal via `..` is impossible.
pub fn download_dir(gallery_path: &Path, id: u64, _title: &str) -> PathBuf {
    gallery_path.join(download_dir_name(id))
}

/// The directory name for a namespaced media id: `{id}` for Pixiv and
/// `{prefix}{native}` for direct-CDN providers (the prefix comes from the
/// provider itself).
fn download_dir_name(id: u64) -> String {
    let provider = crate::media::provider_of(id);
    format!("{}{}", provider.dir_prefix(), crate::media::native_id(id))
}

/// Parse a gallery directory name back into a namespaced media id:
/// `123` → Pixiv id, `tw_456` → Twitter id. Returns `None` for anything else
/// (guards against `..`/path traversal).
fn parse_dir_id(name: &str) -> Option<u64> {
    for provider in [
        crate::media::Provider::Twitter,
        crate::media::Provider::Bilibili,
    ] {
        if let Some(rest) = name.strip_prefix(provider.dir_prefix()) {
            let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            return digits
                .parse::<u64>()
                .ok()
                .map(|native| provider.key(native));
        }
    }
    let digits: String = name.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse::<u64>().ok()
}

/// Migrate an illust's download directory from the OLD `{id}_{title}` format
/// to the new `{id}` format. Looks for a directory matching `<id>_*`; if one
/// exists (and looks like a download dir — it contains image files), renames
/// it to `{id}`. Returns the path of the migrated (or pre-existing) dir, or
/// `None` when nothing is on disk for `id`.
pub fn migrate_download_dir(gallery_path: &Path, id: u64) -> Option<PathBuf> {
    let new_dir = gallery_path.join(download_dir_name(id));
    if new_dir.is_dir() {
        return Some(new_dir);
    }
    // Only Pixiv has legacy `{id}_{title}` dirs; direct-CDN dirs are new.
    if crate::media::provider_of(id) != crate::media::Provider::Pixiv {
        return None;
    }
    let Ok(entries) = std::fs::read_dir(gallery_path) else {
        return None;
    };
    let id_str = id.to_string();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if let Some(rest) = name.strip_prefix(&id_str) {
            // Must be `{id}_...` (an underscore right after the id).
            if !rest.starts_with('_') {
                continue;
            }
            // Only migrate if it actually holds images (avoid clobbering a
            // dir that merely happens to start with the id).
            if dir_has_images(&path) {
                let _ = std::fs::rename(&path, &new_dir);
                return Some(new_dir);
            }
        }
    }
    None
}

/// Scan the gallery directory and collect every illust id that exists on
/// disk. Directory names follow [`download_dir`]'s `{id}_{title}` scheme, so
/// the id is parsed from the leading integer segment (guarding against
/// `..`/path traversal — only entries that parse as a pure u64 prefix count).
///
/// This drives the "Missing" filter (Java semantics: missing = not downloaded
/// locally) and per-tile downloaded checkmarks.
pub fn scan_downloaded_ids(gallery_path: &Path) -> std::collections::HashSet<u64> {
    let mut ids = std::collections::HashSet::new();
    let Ok(entries) = std::fs::read_dir(gallery_path) else {
        return ids;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        // Dir names are `{id}` / `{id}_{title}` (Pixiv) or `tw_{id}` (Twitter).
        if let Some(id) = parse_dir_id(&name) {
            ids.insert(id);
        }
    }
    ids
}

/// The human title embedded in a gallery directory name (the part after the
/// provider id; `tw_{id}_{title}` and `{id}_{title}` both strip correctly).
fn dir_title(name: &str) -> &str {
    let rest = name
        .strip_prefix("tw_")
        .or_else(|| name.strip_prefix("bl_"))
        .unwrap_or(name);
    let digits = rest.chars().take_while(|c| c.is_ascii_digit()).count();
    rest[digits..].trim_start_matches(['_', ' ', '-'])
}

/// A local illust discovered on disk: its id and the sanitized title from the
/// directory name, plus the newest file mtime (for "newest first" sorting).
struct LocalIllust {
    id: u64,
    title: String,
    dir: PathBuf,
    newest_mtime: Option<chrono::DateTime<chrono::Utc>>,
}

/// Scan the gallery directory for downloaded illusts whose ids are NOT in
/// `exclude` (e.g. the bookmark library), sorted newest-first — the "Extra"
/// tab's data: local stuff you downloaded but no longer have bookmarked.
///
/// Returns `(illustrations, dirs)` where `dirs` maps id → download directory
/// so the caller can lazily load local thumbnails.
pub fn scan_local_illusts(
    gallery_path: &Path,
    exclude: &std::collections::HashSet<u64>,
) -> (Vec<Illustration>, std::collections::HashMap<u64, PathBuf>) {
    let mut found: Vec<LocalIllust> = Vec::new();
    let Ok(entries) = std::fs::read_dir(gallery_path) else {
        return (Vec::new(), std::collections::HashMap::new());
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let Some(id) = parse_dir_id(&name) else {
            continue;
        };
        if exclude.contains(&id) {
            continue;
        }
        let title = dir_title(&name);
        let title = if title.is_empty() {
            "untitled".to_string()
        } else {
            title.to_string()
        };
        // Newest file mtime in the dir (newest download first).
        let newest_mtime = std::fs::read_dir(&path)
            .ok()
            .and_then(|files| {
                files
                    .flatten()
                    .filter_map(|f| f.metadata().ok().and_then(|m| m.modified().ok()))
                    .max()
            })
            .map(chrono::DateTime::<chrono::Utc>::from);
        found.push(LocalIllust {
            id,
            title,
            dir: path,
            newest_mtime,
        });
    }
    found.sort_by_key(|l| std::cmp::Reverse(l.newest_mtime));

    let mut illusts = Vec::with_capacity(found.len());
    let mut dirs = std::collections::HashMap::with_capacity(found.len());
    for l in found {
        dirs.insert(l.id, l.dir);
        illusts.push(Illustration::local(l.id, l.title, l.newest_mtime));
    }
    (illusts, dirs)
}

/// The original-URL for every page of an illust, in page order. Falls back to
/// the largest available single image when pagination metadata is absent.
pub fn collect_download_urls(illust: &Illustration) -> Vec<String> {
    if !illust.meta_pages.is_empty() {
        illust
            .meta_pages
            .iter()
            .filter_map(|p| p.image_urls.original.clone())
            .collect()
    } else if let Some(single) = &illust.meta_single_page {
        single.original_image_url.clone().into_iter().collect()
    } else {
        illust
            .image_urls
            .as_ref()
            .and_then(|u| u.large.clone())
            .into_iter()
            .collect()
    }
}

/// The on-disk filename for a page: `{index}.{ext}` (0-based page index)
/// inside the illust's `{id}` directory.
pub fn image_filename(_id: u64, index: usize, _total: usize, ext: &str) -> String {
    format!("{index}.{ext}")
}

/// The file extension for an image URL (query strings stripped, default `jpg`).
pub fn file_extension(url: &str) -> &str {
    url.split('?')
        .next()
        .and_then(|s| s.rsplit('.').next())
        .unwrap_or("jpg")
}

/// Whether an illust is already downloaded on disk under the gallery
/// directory. Requires the `{id}` dir to exist and contain at least as many
/// image files as the illust's page count (so a partially-downloaded
/// multi-page illust is NOT skipped — the downloader resumes it). Robust
/// against missing URL metadata (the old check needed
/// `collect_download_urls`, which library illusts sometimes lack — that's why
/// "download all" kept re-downloading everything), and migrates a legacy
/// `{id}_{title}` dir to the new `{id}` layout.
pub fn is_illust_downloaded(gallery_path: &Path, illust: &Illustration) -> bool {
    match migrate_download_dir(gallery_path, illust.id) {
        Some(dir) => {
            let count = image_file_count(&dir);
            count > 0 && count >= illust.page_count().max(1)
        }
        None => false,
    }
}

/// Whether a directory contains at least one image file (by extension).
fn dir_has_images(dir: &Path) -> bool {
    image_file_count(dir) > 0
}

/// Count image files (by extension) in a directory.
fn image_file_count(dir: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .filter(|e| {
            e.path().is_file()
                && e.path()
                    .extension()
                    .map(|ext| {
                        ext.eq_ignore_ascii_case("jpg")
                            || ext.eq_ignore_ascii_case("jpeg")
                            || ext.eq_ignore_ascii_case("png")
                            || ext.eq_ignore_ascii_case("gif")
                    })
                    .unwrap_or(false)
        })
        .count()
}

/// Locate the on-disk image file for `page` of `id` inside its download
/// directory. Handles the current `{page}.{ext}` naming plus the legacy
/// `{id}.{ext}` (single-page) and `{id}_p{page}.{ext}` (multi-page) names.
pub fn find_page_file(dir: &Path, id: u64, page: usize) -> Option<PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let stem = path.file_stem()?.to_string_lossy().to_string();
        if stem == page.to_string()
            || stem == format!("{id}_p{page}")
            || (page == 0 && stem == id.to_string())
        {
            return Some(path);
        }
    }
    None
}

// ── Pagination ──────────────────────────────────────────────────────────

/// Anything that carries an illust list plus a `next_url` pagination cursor.
pub trait Paginated {
    fn illusts(&self) -> &[Illustration];
    fn next_url(&self) -> &Option<String>;
}

macro_rules! impl_paginated {
    ($ty:ty) => {
        impl Paginated for $ty {
            fn illusts(&self) -> &[Illustration] {
                &self.illusts
            }
            fn next_url(&self) -> &Option<String> {
                &self.next_url
            }
        }
    };
}

impl_paginated!(pixivj_rs::model::SearchedIllusts);
impl_paginated!(pixivj_rs::model::UserIllusts);

/// Collect every page of a paginated response into a flat illust list.
pub async fn paginate<P: Paginated + serde::de::DeserializeOwned>(
    client: &pixivj_rs::api::PixivApiClient,
    first: P,
) -> Result<Vec<Illustration>, pixivj_rs::error::PixivError> {
    let mut all = first.illusts().to_vec();
    let mut next_url = first.next_url().clone();
    while let Some(ref url) = next_url {
        if url.is_empty() {
            break;
        }
        let page: P = client.fetch_by_next_url(url).await?;
        all.extend_from_slice(page.illusts());
        next_url = page.next_url().clone();
    }
    Ok(all)
}

/// Maximum length (in characters) of a sanitized filename segment. Keeps
/// user-controlled Pixiv titles from exceeding filesystem name-length limits.
const MAX_FILENAME_CHARS: usize = 100;

/// Windows device names that cannot be used as file/directory names, even
/// with an extension.
const RESERVED_WINDOWS_NAMES: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Replace filesystem-unsafe characters in a name and guard against the
/// remaining pitfalls:
/// * filesystem metacharacters (`/\:*?"<>|`) → `_`
/// * control characters stripped
/// * length capped at ~100 characters (char-boundary safe)
/// * Windows reserved device names (`CON`, `NUL`, `COM1`, …) defanged
/// * trailing dots/spaces removed (Windows treats them as invalid)
///
/// Callers always prefix results with a numeric id (e.g. `{id}_{title}`), so
/// path traversal via `..` is not possible here.
pub fn sanitize_filename(name: &str) -> String {
    let mut out: String = name
        .chars()
        .filter(|c| !c.is_control())
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c => c,
        })
        .collect();
    if out.chars().count() > MAX_FILENAME_CHARS {
        out = out.chars().take(MAX_FILENAME_CHARS).collect();
    }
    let trimmed = out.trim_end_matches(['.', ' ']);
    if trimmed.to_uppercase().as_str().is_reserved_windows_name() {
        out = format!("_{trimmed}");
    } else {
        out = trimmed.to_string();
    }
    if out.is_empty() {
        out = "untitled".to_string();
    }
    out
}

trait ReservedWindowsName {
    fn is_reserved_windows_name(&self) -> bool;
}

impl ReservedWindowsName for str {
    fn is_reserved_windows_name(&self) -> bool {
        // A reserved name is one of the table (an extension like `CON.txt`
        // is also reserved on Windows, but callers here strip extensions).
        RESERVED_WINDOWS_NAMES.contains(&self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaces_metacharacters() {
        assert_eq!(
            sanitize_filename("a/b\\c:d*e?f\"g<h>i|j"),
            "a_b_c_d_e_f_g_h_i_j"
        );
    }

    #[test]
    fn strips_control_characters() {
        assert_eq!(sanitize_filename("a\u{1}b\u{7}c"), "abc");
    }

    #[test]
    fn caps_length_at_char_boundary() {
        let long = "x".repeat(500);
        assert_eq!(sanitize_filename(&long).chars().count(), 100);
        // Multibyte chars are never split.
        let long_utf8 = "é".repeat(300);
        let capped = sanitize_filename(&long_utf8);
        assert_eq!(capped.chars().count(), 100);
        assert!(capped.is_char_boundary(capped.len()));
    }

    #[test]
    fn handles_windows_reserved_names() {
        assert_eq!(sanitize_filename("CON"), "_CON");
        assert_eq!(sanitize_filename("nul"), "_nul");
        assert_eq!(sanitize_filename("LPT1"), "_LPT1");
        assert_eq!(sanitize_filename("not_reserved"), "not_reserved");
    }

    #[test]
    fn trims_trailing_dots_and_spaces() {
        assert_eq!(sanitize_filename("trailing. "), "trailing");
        assert_eq!(sanitize_filename("ok"), "ok");
    }

    #[test]
    fn empty_becomes_untitled() {
        assert_eq!(sanitize_filename("\u{1}\u{2}\u{3}"), "untitled");
    }

    fn single_page_illust(id: u64, url: &str) -> Illustration {
        Illustration {
            id,
            title: Some(format!("t{id}")),
            illust_type: None,
            image_urls: Some(pixivj_rs::model::IllustImageUrls {
                square_medium: Some(url.into()),
                medium: Some(url.into()),
                large: Some(url.into()),
            }),
            caption: None,
            restrict: None,
            user: None,
            tags: vec![],
            tools: vec![],
            create_date: None,
            page_count: Some(1),
            width: None,
            height: None,
            sanity_level: None,
            x_restrict: None,
            series: None,
            meta_single_page: Some(pixivj_rs::model::MetaSinglePage {
                original_image_url: Some(url.into()),
            }),
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
    fn is_illust_downloaded_true_when_all_files_present() {
        let tmp = tempfile::tempdir().unwrap();
        let gallery = tmp.path();
        let illust = single_page_illust(42, "https://i.pximg.net/x/42.jpg");
        let dir = download_dir(gallery, 42, illust.title());
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("0.jpg"), b"data").unwrap();
        assert!(is_illust_downloaded(gallery, &illust));
    }

    #[test]
    fn is_illust_downloaded_false_when_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let gallery = tmp.path();
        // Nothing downloaded yet.
        let illust = single_page_illust(42, "https://i.pximg.net/x/42.jpg");
        assert!(!is_illust_downloaded(gallery, &illust));
        // Dir exists but empty → not downloaded.
        let dir = download_dir(gallery, 42, illust.title());
        std::fs::create_dir_all(&dir).unwrap();
        assert!(!is_illust_downloaded(gallery, &illust));
    }

    #[test]
    fn is_illust_downloaded_false_for_multi_page_partial() {
        let tmp = tempfile::tempdir().unwrap();
        let gallery = tmp.path();
        let mut illust = single_page_illust(7, "https://i.pximg.net/x/7_p0.jpg");
        illust.page_count = Some(2);
        let dir = download_dir(gallery, 7, illust.title());
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("0.jpg"), b"data").unwrap();
        // Only page 0 exists → not fully downloaded.
        assert!(!is_illust_downloaded(gallery, &illust));
        std::fs::write(dir.join("1.jpg"), b"data").unwrap();
        assert!(is_illust_downloaded(gallery, &illust));
    }

    #[test]
    fn legacy_title_dir_is_migrated_to_id() {
        let tmp = tempfile::tempdir().unwrap();
        let gallery = tmp.path();
        let illust = single_page_illust(42, "https://i.pximg.net/x/42.jpg");
        // Old format: `{id}_{title}` with a valid image inside.
        let old = gallery.join("42_my_artwork");
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(old.join("42.jpg"), b"data").unwrap();

        assert!(is_illust_downloaded(gallery, &illust), "old dir counts");
        // The old dir was renamed to the new `{id}` layout.
        assert!(gallery.join("42").is_dir(), "renamed to {{id}}");
        assert!(!old.exists(), "old dir moved away");
    }

    #[test]
    fn download_dir_is_just_the_id() {
        let tmp = tempfile::tempdir().unwrap();
        let gallery = tmp.path();
        assert_eq!(
            download_dir(gallery, 123, "any title with spaces/and..dots"),
            gallery.join("123")
        );
        assert_eq!(
            image_filename(123, 0, 1, "jpg"),
            "0.jpg",
            "page index is the filename"
        );
    }

    #[test]
    fn find_page_file_matches_new_and_legacy_names() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();

        // New naming: `{page}.{ext}`.
        std::fs::write(dir.join("0.jpg"), b"a").unwrap();
        std::fs::write(dir.join("1.png"), b"b").unwrap();
        assert_eq!(find_page_file(dir, 7, 0), Some(dir.join("0.jpg")));
        assert_eq!(find_page_file(dir, 7, 1), Some(dir.join("1.png")));

        // Legacy multi-page: `{id}_p{page}.{ext}`.
        let legacy = tempfile::tempdir().unwrap();
        std::fs::write(legacy.path().join("7_p2.jpg"), b"c").unwrap();
        assert_eq!(
            find_page_file(legacy.path(), 7, 2),
            Some(legacy.path().join("7_p2.jpg"))
        );

        // Legacy single-page: `{id}.{ext}` (page 0).
        let single = tempfile::tempdir().unwrap();
        std::fs::write(single.path().join("9.jpg"), b"d").unwrap();
        assert_eq!(
            find_page_file(single.path(), 9, 0),
            Some(single.path().join("9.jpg"))
        );
        // A page that doesn't exist on disk → None.
        assert!(find_page_file(dir, 7, 5).is_none());
    }
}

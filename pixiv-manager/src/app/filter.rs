use pixivj_rs::model::Illustration;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ── Tristate ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Tristate {
    #[default]
    Indeterminate,
    Selected,
    Deselected,
}

impl Tristate {
    pub fn cycle(self) -> Self {
        match self {
            Tristate::Indeterminate => Tristate::Selected,
            Tristate::Selected => Tristate::Deselected,
            Tristate::Deselected => Tristate::Indeterminate,
        }
    }

    pub fn passes(&self, value: bool) -> bool {
        match self {
            Tristate::Indeterminate => true,
            Tristate::Selected => value,
            Tristate::Deselected => !value,
        }
    }
}

// ── Filter mode ───────────────────────────────────────────────────────

/// How a tab's list is filtered and sorted. The two modes are mutually
/// exclusive: `Classic` uses the search box + option/tag/tool controls + sort
/// buttons, `Query` uses a single Danbooru-style text query
/// (see [`crate::app::query`]) and ignores the classic controls.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum FilterMode {
    #[default]
    Classic,
    Query,
}

// ── Source Filter ─────────────────────────────────────────────────────

/// Which provider sources pass the filter (the mixed Bookmarks tab shows
/// Pixiv + Twitter + Bilibili items side by side). Every source starts on —
/// unchecking chips narrows the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceFilter {
    pub pixiv: bool,
    pub twitter: bool,
    pub bilibili: bool,
}

impl Default for SourceFilter {
    fn default() -> Self {
        Self {
            pixiv: true,
            twitter: true,
            bilibili: true,
        }
    }
}

impl SourceFilter {
    /// Whether every source is allowed (the filter is then a no-op).
    pub fn all(&self) -> bool {
        self.pixiv && self.twitter && self.bilibili
    }

    /// Whether an item from `provider` passes.
    pub fn passes(&self, provider: crate::media::Provider) -> bool {
        match provider {
            crate::media::Provider::Pixiv => self.pixiv,
            crate::media::Provider::Twitter => self.twitter,
            crate::media::Provider::Bilibili => self.bilibili,
        }
    }
}

/// Which source chip a toggle flips.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKey {
    Pixiv,
    Twitter,
    Bilibili,
}

// ── Filter State ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OptionFilters {
    pub r18: Tristate,
    pub ai: Tristate,
    pub bookmarked: Tristate,
    pub landscape: Tristate,
    pub visible: Tristate,
    pub restricted: Tristate,
    pub missing: Tristate,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
#[allow(dead_code)] // search_tools will be wired to UI
pub struct FilterState {
    /// Classic controls or the text query.
    pub mode: FilterMode,
    /// The text query of [`FilterMode::Query`].
    pub query_text: String,
    pub search_text: String,
    /// Match the query against tag names (incl. translated names).
    pub search_tags: bool,
    /// Match the query against the author's name.
    pub search_author: bool,
    /// Match the query against the title.
    pub search_title: bool,
    /// Match the query against the caption.
    pub search_caption: bool,
    /// Match the query against the illust id (prefix).
    pub search_id: bool,
    pub search_ignore_case: bool,
    pub search_tools: bool,
    pub options: OptionFilters,
    /// Which provider sources pass (the mixed Bookmarks tab).
    pub sources: SourceFilter,
    pub selected_tags: Vec<String>,
    pub tool_states: HashMap<String, Tristate>,
    /// Text currently typed in the tag-filter input.
    pub tag_input: String,
}

impl Default for FilterState {
    fn default() -> Self {
        Self {
            mode: FilterMode::Classic,
            query_text: String::new(),
            search_text: String::new(),
            search_tags: true,
            search_author: true,
            search_title: true,
            search_caption: true,
            search_id: true,
            search_ignore_case: true,
            search_tools: false,
            options: OptionFilters::default(),
            sources: SourceFilter::default(),
            selected_tags: Vec::new(),
            tool_states: HashMap::new(),
            tag_input: String::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OptionFilterKey {
    R18,
    Ai,
    Bookmarked,
    Landscape,
    Visible,
    Restricted,
    Missing,
}

/// Which search-target field a toggle affects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchTargetKey {
    Tags,
    Author,
    Title,
    Caption,
    Id,
}

impl FilterState {
    /// Whether any filter constrains results (drives the "no matches — clear
    /// filters?" empty state).
    pub fn is_active(&self) -> bool {
        if self.mode == FilterMode::Query {
            return !crate::app::query::Query::parse(&self.query_text).is_empty();
        }
        let OptionFilters {
            r18,
            ai,
            bookmarked,
            landscape,
            visible,
            restricted,
            missing,
        } = &self.options;
        let option_set = [r18, ai, bookmarked, landscape, visible, restricted, missing]
            .iter()
            .any(|t| **t != Tristate::Indeterminate);
        option_set
            || !self.sources.all()
            || !self.selected_tags.is_empty()
            || self
                .tool_states
                .values()
                .any(|t| *t != Tristate::Indeterminate)
            || !self.search_text.trim().is_empty()
    }

    /// Pure function: filter illustrations by all active filters. `downloaded`
    /// is the set of illust ids found on disk in the gallery directory — it
    /// drives the "Missing" tristate (Java semantics: a missing illust is one
    /// *not downloaded locally*). An empty set means "unknown", so Missing
    /// falls back to "no thumbnail URL in the API payload".
    pub fn apply(
        &self,
        illusts: &[Illustration],
        downloaded: &std::collections::HashSet<u64>,
    ) -> Vec<Illustration> {
        if self.mode == FilterMode::Query {
            return self.apply_query(illusts, downloaded);
        }
        illusts
            .iter()
            .filter(|i| self.matches(i, downloaded))
            .cloned()
            .collect()
    }

    /// Query mode: only the text query filters. The one exception is the
    /// global "hide bookmarked items" preference (seeded into
    /// `options.bookmarked`), which keeps applying unless the query itself
    /// talks about `is:bookmarked`.
    fn apply_query(
        &self,
        illusts: &[Illustration],
        downloaded: &std::collections::HashSet<u64>,
    ) -> Vec<Illustration> {
        let query = crate::app::query::Query::parse(&self.query_text);
        let ctx = crate::app::query::Context {
            now: chrono::Utc::now(),
            downloaded,
        };
        let bookmarked = if query.mentions_bookmarked() {
            Tristate::Indeterminate
        } else {
            self.options.bookmarked
        };
        illusts
            .iter()
            .filter(|i| bookmarked.passes(i.bookmarked()) && query.matches(i, &ctx))
            .cloned()
            .collect()
    }

    fn matches(&self, i: &Illustration, downloaded: &std::collections::HashSet<u64>) -> bool {
        // Source filter (mixed tabs): the provider must be allowed.
        if !self.sources.all() && !self.sources.passes(crate::media::provider_of(i.id)) {
            return false;
        }
        // Tristate option filters
        if !self.options.r18.passes(i.is_r18()) {
            return false;
        }
        if !self.options.ai.passes(i.is_ai()) {
            return false;
        }
        if !self.options.bookmarked.passes(i.bookmarked()) {
            return false;
        }
        if !self.options.landscape.passes(i.is_landscape()) {
            return false;
        }
        if !self.options.visible.passes(i.is_visible()) {
            return false;
        }
        if !self.options.restricted.passes(i.is_restricted()) {
            return false;
        }
        // Missing = not downloaded locally (Java semantics); when the
        // downloaded set is unknown (empty), fall back to "no thumbnail URL".
        let missing = if downloaded.is_empty() {
            i.image_urls
                .as_ref()
                .and_then(|u| u.medium.as_ref())
                .is_none()
        } else {
            !downloaded.contains(&i.id)
        };
        if !self.options.missing.passes(missing) {
            return false;
        }

        // Tag filter: all selected tags must be present (name or translated).
        if !self.selected_tags.is_empty() {
            for tag in &self.selected_tags {
                let found = i
                    .tags
                    .iter()
                    .any(|t| &t.name == tag || t.translated_name.as_deref() == Some(tag));
                if !found {
                    return false;
                }
            }
        }

        // Tool filter: all selected tools must be present
        for (tool, state) in &self.tool_states {
            if *state == Tristate::Selected && !i.tools.iter().any(|t| t == tool) {
                return false;
            }
            if *state == Tristate::Deselected && i.tools.iter().any(|t| t == tool) {
                return false;
            }
        }

        // Text search
        if !self.search_text.is_empty() {
            let needle = if self.search_ignore_case {
                self.search_text.to_lowercase()
            } else {
                self.search_text.clone()
            };
            let mut found = false;
            if self.search_tags {
                found |= i.tags.iter().any(|t| {
                    let name = if self.search_ignore_case {
                        t.name.to_lowercase()
                    } else {
                        t.name.clone()
                    };
                    name.contains(&needle)
                        || t.translated_name
                            .as_deref()
                            .map(|tn| {
                                if self.search_ignore_case {
                                    tn.to_lowercase().contains(&needle)
                                } else {
                                    tn.contains(&needle)
                                }
                            })
                            .unwrap_or(false)
                });
            }
            if self.search_author {
                if let Some(ref author) = i.user {
                    let name = if self.search_ignore_case {
                        author.name.to_lowercase()
                    } else {
                        author.name.clone()
                    };
                    found |= name.contains(&needle);
                }
            }
            if self.search_title {
                if let Some(title) = i.title.as_deref() {
                    let title = if self.search_ignore_case {
                        title.to_lowercase()
                    } else {
                        title.to_string()
                    };
                    found |= title.contains(&needle);
                }
            }
            if self.search_caption {
                if let Some(caption) = i.caption.as_deref() {
                    let caption = if self.search_ignore_case {
                        caption.to_lowercase()
                    } else {
                        caption.to_string()
                    };
                    found |= caption.contains(&needle);
                }
            }
            if self.search_id {
                let id_str = i.id.to_string();
                found |= id_str.starts_with(&needle);
            }
            if !found {
                return false;
            }
        }

        true
    }
}

// ── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use pixivj_rs::model::*;

    fn test_illust(id: u64) -> Illustration {
        Illustration {
            id,
            title: Some(format!("illust {}", id)),
            illust_type: None,
            image_urls: Some(IllustImageUrls {
                square_medium: Some("thumb.jpg".into()),
                medium: Some("thumb.jpg".into()),
                large: Some("large.jpg".into()),
            }),
            caption: None,
            restrict: None,
            user: Some(Author {
                id: 1,
                profile_image_urls: None,
                name: "artist".into(),
                account: None,
                comment: None,
                is_followed: None,
            }),
            tags: vec![],
            tools: vec![],
            create_date: None,
            page_count: Some(1),
            width: Some(800),
            height: Some(600),
            sanity_level: None,
            x_restrict: None,
            series: None,
            meta_single_page: None,
            meta_pages: vec![],
            total_view: Some(100),
            total_bookmarks: Some(10),
            is_bookmarked: Some(false),
            visible: Some(true),
            is_muted: None,
            illust_ai_type: None,
            provider_data: None,
        }
    }

    #[test]
    fn tristate_cycle() {
        assert_eq!(Tristate::Indeterminate.cycle(), Tristate::Selected);
        assert_eq!(Tristate::Selected.cycle(), Tristate::Deselected);
        assert_eq!(Tristate::Deselected.cycle(), Tristate::Indeterminate);
    }

    #[test]
    fn tristate_passes() {
        assert!(Tristate::Indeterminate.passes(true));
        assert!(Tristate::Indeterminate.passes(false));
        assert!(Tristate::Selected.passes(true));
        assert!(!Tristate::Selected.passes(false));
        assert!(!Tristate::Deselected.passes(true));
        assert!(Tristate::Deselected.passes(false));
    }

    #[test]
    fn filter_empty_passes_all() {
        let filter = FilterState::default();
        let illusts = vec![test_illust(1), test_illust(2)];
        let result = filter.apply(&illusts, &Default::default());
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn filter_r18_selected() {
        let mut filter = FilterState::default();
        filter.options.r18 = Tristate::Selected;
        let mut illusts = vec![test_illust(1), test_illust(2)];
        illusts[0].x_restrict = Some(1); // R-18
        let result = filter.apply(&illusts, &Default::default());
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, 1);
    }

    #[test]
    fn filter_by_tag() {
        let filter = FilterState {
            selected_tags: vec!["cat".into()],
            ..Default::default()
        };
        let mut illusts = vec![test_illust(1), test_illust(2)];
        illusts[0].tags = vec![Tag {
            name: "cat".into(),
            translated_name: None,
        }];
        let result = filter.apply(&illusts, &Default::default());
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, 1);
    }

    #[test]
    fn filter_by_search_text() {
        let filter = FilterState {
            search_text: "artist".into(),
            ..Default::default()
        };
        let illusts = vec![test_illust(1), test_illust(2)];
        let result = filter.apply(&illusts, &Default::default());
        assert_eq!(result.len(), 2); // both have author "artist"
    }

    #[test]
    fn filter_landscape() {
        let mut filter = FilterState::default();
        filter.options.landscape = Tristate::Selected;
        let illusts = vec![test_illust(1)]; // 800x600 = landscape
        let result = filter.apply(&illusts, &Default::default());
        assert_eq!(result.len(), 1);
    }

    #[test]
    #[allow(clippy::field_reassign_with_default)]
    fn search_matches_title_caption_and_id() {
        let mut filter = FilterState::default();
        filter.search_tags = false;
        filter.search_author = false;

        // Title match.
        let mut title_illust = test_illust(10);
        title_illust.title = Some("My beautiful beach day".into());
        filter.search_text = "beach".into();
        assert_eq!(
            filter
                .apply(&[title_illust.clone()], &Default::default())
                .len(),
            1
        );

        // Caption match.
        let mut caption_illust = test_illust(11);
        caption_illust.caption = Some("drawn with love".into());
        filter.search_text = "drawn with".into();
        assert_eq!(
            filter
                .apply(
                    &[title_illust.clone(), caption_illust.clone()],
                    &Default::default()
                )
                .len(),
            1
        );

        // ID prefix match.
        filter.search_caption = false;
        filter.search_title = false;
        filter.search_id = true;
        filter.search_text = "11".into();
        assert_eq!(
            filter
                .apply(&[title_illust, caption_illust], &Default::default())
                .len(),
            1
        );
    }

    #[test]
    fn tag_match_uses_translated_name() {
        let filter = FilterState {
            selected_tags: vec!["翻訳された".into()],
            ..Default::default()
        };
        let mut illust = test_illust(1);
        illust.tags = vec![Tag {
            name: "original".into(),
            translated_name: Some("翻訳された".into()),
        }];
        assert_eq!(filter.apply(&[illust], &Default::default()).len(), 1);
    }

    #[test]
    fn missing_filter_uses_downloaded_set() {
        let mut filter = FilterState::default();
        filter.options.missing = Tristate::Selected;

        let mut with_thumb = test_illust(1);
        with_thumb.image_urls = Some(IllustImageUrls {
            square_medium: Some("t.jpg".into()),
            medium: Some("t.jpg".into()),
            large: Some("l.jpg".into()),
        });

        // Empty downloaded set → Missing falls back to "no thumbnail URL".
        assert_eq!(
            filter
                .apply(&[with_thumb.clone()], &Default::default())
                .len(),
            0,
            "has a thumbnail → not missing (downloaded unknown)"
        );
        let no_thumb = {
            let mut i = test_illust(2);
            i.image_urls = None;
            i
        };
        assert_eq!(
            filter.apply(&[no_thumb], &Default::default()).len(),
            1,
            "no thumbnail → missing when downloaded set unknown"
        );

        // With a known downloaded set, Missing = "not downloaded locally".
        let downloaded = [2u64].into_iter().collect();
        assert_eq!(
            filter.apply(&[with_thumb.clone()], &downloaded).len(),
            1,
            "has thumbnail but not on disk → missing"
        );
        let mut on_disk = test_illust(3);
        on_disk.image_urls = Some(IllustImageUrls {
            square_medium: Some("t.jpg".into()),
            medium: Some("t.jpg".into()),
            large: Some("l.jpg".into()),
        });
        let downloaded = [3u64].into_iter().collect();
        assert_eq!(
            filter.apply(&[on_disk], &downloaded).len(),
            0,
            "downloaded locally → not missing"
        );
    }

    #[test]
    fn source_filter_passes_by_provider() {
        // The mixed Bookmarks tab: filtering a source drops its items.
        let filter = FilterState {
            sources: SourceFilter {
                pixiv: true,
                twitter: false,
                bilibili: true,
            },
            ..Default::default()
        };
        let pixiv = test_illust(1); // native id → Pixiv
        let tweet = test_illust(crate::media::twitter_key(2));
        let dynamic = test_illust(crate::media::bilibili_key(3));
        let result = filter.apply(&[pixiv, tweet, dynamic], &Default::default());
        assert_eq!(result.len(), 2, "the tweet is filtered out");
        assert!(result.iter().all(|i| !crate::media::is_twitter(i.id)));

        // Default filter passes everything (no-op).
        let filter = FilterState::default();
        let illusts = vec![test_illust(1), test_illust(crate::media::twitter_key(2))];
        assert_eq!(filter.apply(&illusts, &Default::default()).len(), 2);
    }
}

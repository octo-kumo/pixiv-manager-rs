// Data models for the Pixiv API.
// Ported from the Java pixivj library — all 50 model types.

use chrono::{DateTime, Utc};

// ── Enums ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub enum IllustType {
    #[serde(rename = "illust")]
    Illust,
    #[serde(rename = "ugoira")]
    Ugoira,
    #[serde(rename = "manga")]
    Manga,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub enum FilterType {
    #[serde(rename = "for_ios")]
    ForIos,
    #[serde(rename = "for_android")]
    ForAndroid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub enum FilterMode {
    #[serde(rename = "day")]
    Day,
    #[serde(rename = "day_male")]
    DayMale,
    #[serde(rename = "day_female")]
    DayFemale,
    #[serde(rename = "week_original")]
    WeekOriginal,
    #[serde(rename = "week_rookie")]
    WeekRookie,
    #[serde(rename = "month_rookie")]
    MonthRookie,
    #[serde(rename = "week")]
    Week,
    #[serde(rename = "month")]
    Month,
    #[serde(rename = "day_r18")]
    DayR18,
    #[serde(rename = "week_r18")]
    WeekR18,
    #[serde(rename = "week_r18g")]
    WeekR18g,
    #[serde(rename = "day_male_r18")]
    DayMaleR18,
    #[serde(rename = "day_female_r18")]
    DayFemaleR18,
    #[serde(rename = "day_manga")]
    DayManga,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub enum SearchTarget {
    #[serde(rename = "partial_match_for_tags")]
    PartialMatchForTags,
    #[serde(rename = "exact_match_for_tags")]
    ExactMatchForTags,
    #[serde(rename = "text")]
    Text,
    #[serde(rename = "keyword_and_tags")]
    KeywordAndTags,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub enum SortOption {
    #[serde(rename = "date_asc")]
    DateAsc,
    #[serde(rename = "date_desc")]
    DateDesc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Restrict {
    Public,
    Private,
}

// ── Image URLs ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct IllustImageUrls {
    #[serde(rename = "square_medium")]
    pub square_medium: Option<String>,
    pub medium: Option<String>,
    pub large: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct MetaPageImageUrls {
    #[serde(rename = "square_medium")]
    pub square_medium: Option<String>,
    pub medium: Option<String>,
    pub large: Option<String>,
    pub original: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct MetaSinglePage {
    #[serde(rename = "original_image_url")]
    pub original_image_url: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct MetaPage {
    #[serde(rename = "image_urls")]
    pub image_urls: MetaPageImageUrls,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct AuthorProfileImageUrls {
    pub medium: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct ProfileImageUrls {
    #[serde(rename = "px_16x16")]
    pub px_16x16: Option<String>,
    #[serde(rename = "px_50x50")]
    pub px_50x50: Option<String>,
    #[serde(rename = "px_170x170")]
    pub px_170x170: Option<String>,
}

// ── Core Types ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct Tag {
    pub name: String,
    #[serde(rename = "translated_name")]
    pub translated_name: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct Series {
    pub id: Option<u64>,
    pub title: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct Author {
    #[serde(deserialize_with = "deserialize_id_as_u64", default)]
    pub id: u64,
    #[serde(rename = "profile_image_urls")]
    pub profile_image_urls: Option<AuthorProfileImageUrls>,
    pub name: String,
    pub account: Option<String>,
    pub comment: Option<String>,
    #[serde(rename = "is_followed")]
    pub is_followed: Option<bool>,
}

impl Author {
    pub fn followed(&self) -> bool {
        self.is_followed.unwrap_or(false)
    }
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct Illustration {
    #[serde(deserialize_with = "deserialize_id_as_u64", default)]
    pub id: u64,
    pub title: Option<String>,
    #[serde(rename = "type")]
    pub illust_type: Option<IllustType>,
    #[serde(rename = "image_urls")]
    pub image_urls: Option<IllustImageUrls>,
    pub caption: Option<String>,
    pub restrict: Option<i32>,
    pub user: Option<Author>,
    pub tags: Vec<Tag>,
    pub tools: Vec<String>,
    #[serde(rename = "create_date")]
    pub create_date: Option<DateTime<Utc>>,
    #[serde(rename = "page_count")]
    pub page_count: Option<i32>,
    pub width: Option<i32>,
    pub height: Option<i32>,
    #[serde(rename = "sanity_level")]
    pub sanity_level: Option<i32>,
    #[serde(rename = "x_restrict")]
    pub x_restrict: Option<i32>,
    pub series: Option<Series>,
    #[serde(rename = "meta_single_page")]
    pub meta_single_page: Option<MetaSinglePage>,
    #[serde(rename = "meta_pages")]
    pub meta_pages: Vec<MetaPage>,
    #[serde(rename = "total_view")]
    pub total_view: Option<u64>,
    #[serde(rename = "total_bookmarks")]
    pub total_bookmarks: Option<u64>,
    #[serde(rename = "is_bookmarked")]
    pub is_bookmarked: Option<bool>,
    pub visible: Option<bool>,
    #[serde(rename = "is_muted")]
    pub is_muted: Option<bool>,
    #[serde(rename = "illust_ai_type")]
    pub illust_ai_type: Option<i32>,
    /// Provider-specific extras that the shared (Pixiv-shaped) schema has no
    /// home for. Persisted with the item so library entries keep working.
    #[serde(
        rename = "provider_data",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub provider_data: Option<ProviderData>,
}

/// Extras for non-Pixiv providers (see [`Illustration::provider_data`]).
#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct ProviderData {
    /// Bilibili: the comment-thread id (`basic.comment_id_str`), which is not
    /// the dynamic id and is required by the reply API.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment_oid: Option<String>,
}

impl Illustration {
    pub fn is_ai(&self) -> bool {
        self.illust_ai_type == Some(2)
    }
    pub fn bookmarked(&self) -> bool {
        self.is_bookmarked.unwrap_or(false)
    }
    pub fn is_visible(&self) -> bool {
        self.visible.unwrap_or(true)
    }
    pub fn is_landscape(&self) -> bool {
        self.width.unwrap_or(0) > self.height.unwrap_or(0)
    }
    pub fn is_r18(&self) -> bool {
        self.x_restrict.unwrap_or(0) != 0
    }
    pub fn is_restricted(&self) -> bool {
        self.restrict.unwrap_or(0) != 0
    }
    /// The number of pages (Pixiv always sends it; defaults to 1 when absent).
    pub fn page_count(&self) -> usize {
        self.page_count.unwrap_or(1).max(1) as usize
    }
    /// The display title (Pixiv always sends it; defaults to "untitled").
    pub fn title(&self) -> &str {
        self.title.as_deref().unwrap_or("untitled")
    }
    /// A synthetic illustration reconstructed from a local download directory
    /// (no Pixiv metadata available — used by the Extra tab's local scan).
    pub fn local(
        id: u64,
        title: String,
        create_date: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Self {
        Self {
            id,
            title: Some(title),
            create_date,
            ..Self::default_local()
        }
    }

    /// The all-None skeleton for locally-synthesized illustrations.
    pub fn default_local() -> Self {
        Self {
            id: 0,
            title: None,
            illust_type: None,
            image_urls: None,
            caption: None,
            restrict: None,
            user: None,
            tags: Vec::new(),
            tools: Vec::new(),
            create_date: None,
            page_count: Some(1),
            width: None,
            height: None,
            sanity_level: None,
            x_restrict: None,
            series: None,
            meta_single_page: None,
            meta_pages: Vec::new(),
            total_view: None,
            total_bookmarks: None,
            is_bookmarked: None,
            visible: None,
            is_muted: None,
            illust_ai_type: None,
            provider_data: None,
        }
    }
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct IllustDetail {
    pub illust: Illustration,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct Comment {
    #[serde(deserialize_with = "deserialize_id_as_u64", default)]
    pub id: u64,
    pub comment: String,
    pub date: Option<DateTime<Utc>>,
    #[serde(rename = "has_replies")]
    pub has_replies: Option<bool>,
    pub user: Option<Author>,
    /// Pixiv stamp (emoji-like image) attached to the comment, if any.
    pub stamp: Option<CommentStamp>,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct CommentStamp {
    #[serde(rename = "stamp_id", default)]
    pub stamp_id: Option<i64>,
    #[serde(rename = "stamp_url")]
    pub stamp_url: Option<String>,
}

#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct Comments {
    pub comments: Vec<Comment>,
    #[serde(rename = "next_url")]
    pub next_url: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct User {
    #[serde(rename = "profile_image_urls")]
    pub profile_image_urls: Option<ProfileImageUrls>,
    #[serde(deserialize_with = "deserialize_id_as_u64", default)]
    pub id: u64,
    pub name: String,
    pub account: Option<String>,
    #[serde(rename = "mail_address")]
    pub mail_address: Option<String>,
    #[serde(rename = "is_premium")]
    pub is_premium: Option<bool>,
    #[serde(rename = "x_restrict")]
    pub x_restrict: Option<i32>,
    #[serde(rename = "is_mail_authorized")]
    pub is_mail_authorized: Option<bool>,
}

/// Pixiv sometimes returns IDs as strings in JSON. Handle both, erroring on
/// garbage instead of silently mapping it to id 0.
fn deserialize_id_as_u64<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    use serde::Deserialize;
    let v = serde_json::Value::deserialize(deserializer)?;
    match v {
        serde_json::Value::Number(n) => n
            .as_u64()
            .ok_or_else(|| Error::custom("expected non-negative id")),
        serde_json::Value::String(s) => s.parse().map_err(|_| Error::custom("invalid id")),
        _ => Err(Error::custom("expected number or string for id")),
    }
}

// ── Response Wrappers ──────────────────────────────────────────────────

#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct RankedIllusts {
    pub illusts: Vec<Illustration>,
    #[serde(rename = "next_url")]
    pub next_url: Option<String>,
}

#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct RecommendedIllusts {
    pub illusts: Vec<Illustration>,
    #[serde(rename = "ranking_illusts")]
    pub ranking_illusts: Option<Vec<Illustration>>,
    #[serde(rename = "contest_exists")]
    pub contest_exists: Option<bool>,
    #[serde(rename = "privacy_policy")]
    pub privacy_policy: Option<PrivacyPolicy>,
    #[serde(rename = "next_url")]
    pub next_url: Option<String>,
}

#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct SearchedIllusts {
    pub illusts: Vec<Illustration>,
    #[serde(rename = "next_url")]
    pub next_url: Option<String>,
    #[serde(rename = "search_span_limit")]
    pub search_span_limit: Option<i32>,
}

#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct Autocomplete {
    pub tags: Vec<Tag>,
}

#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct UserIllusts {
    pub user: Option<User>,
    pub illusts: Vec<Illustration>,
    #[serde(rename = "next_url")]
    pub next_url: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct UserDetail {
    pub user: Author,
    pub profile: Profile,
    #[serde(rename = "profile_publicity")]
    pub profile_publicity: ProfilePublicity,
    pub workspace: Option<Workspace>,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct PrivacyPolicy {
    pub version: Option<String>,
    pub message: Option<String>,
    pub url: Option<String>,
}

// ── Profile / Workspace ────────────────────────────────────────────────

#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct Profile {
    pub webpage: Option<String>,
    pub gender: Option<String>,
    pub birth: Option<String>,
    #[serde(rename = "birth_day")]
    pub birth_day: Option<String>,
    #[serde(rename = "birth_year")]
    pub birth_year: Option<i32>,
    pub region: Option<String>,
    #[serde(rename = "address_id")]
    pub address_id: Option<i32>,
    #[serde(rename = "background_image_url")]
    pub background_image_url: Option<String>,
    #[serde(rename = "background_is_public")]
    pub background_is_public: Option<bool>,
    pub job: Option<String>,
    pub pawoo: Option<bool>,
    #[serde(rename = "total_illust_bookmarks_public", default)]
    pub total_illust_bookmarks_public: Option<u64>,
}

#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct ProfilePublicity {
    pub gender: Option<String>,
    pub region: Option<String>,
    #[serde(rename = "birth_day")]
    pub birth_day: Option<String>,
    #[serde(rename = "birth_year")]
    pub birth_year: Option<String>,
    pub job: Option<String>,
    pub pawoo: Option<bool>,
}

#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct Workspace {
    pub pc: Option<String>,
    pub monitor: Option<String>,
    pub tool: Option<String>,
    pub scanner: Option<String>,
    pub tablet: Option<String>,
    pub mouse: Option<String>,
    pub printer: Option<String>,
    pub desktop: Option<String>,
    pub music: Option<String>,
    pub desk: Option<String>,
    pub chair: Option<String>,
    pub comment: Option<String>,
    #[serde(rename = "workspace_image_url")]
    pub workspace_image_url: Option<String>,
}

// ── Filter / Query Types ──────────────────────────────────────────────

#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct RankedIllustsFilter {
    pub filter: Option<FilterType>,
    pub mode: Option<FilterMode>,
    pub date: Option<String>,
    pub offset: Option<i32>,
}

#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct RecommendedIllustsFilter {
    pub filter: Option<FilterType>,
    #[serde(rename = "min_bookmark_id_for_recent_illust")]
    pub min_bookmark_id_for_recent_illust: Option<String>,
    #[serde(rename = "max_bookmark_id_for_recommend")]
    pub max_bookmark_id_for_recommend: Option<String>,
    pub offset: Option<i32>,
    #[serde(rename = "include_ranking_illusts")]
    pub include_ranking_illusts: Option<bool>,
    #[serde(rename = "include_privacy_policy")]
    pub include_privacy_policy: Option<bool>,
}

#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct SearchedIllustsFilter {
    pub filter: Option<FilterType>,
    #[serde(rename = "include_translated_tag_results")]
    pub include_translated_tag_results: Option<bool>,
    #[serde(rename = "merge_plain_keyword_results")]
    pub merge_plain_keyword_results: Option<bool>,
    pub word: String,
    pub sort: Option<SortOption>,
    pub offset: Option<i32>,
    #[serde(rename = "search_target")]
    pub search_target: Option<SearchTarget>,
}

impl SearchedIllustsFilter {
    pub fn new(word: String) -> Self {
        Self {
            word,
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct IllustCommentsFilter {
    #[serde(rename = "illust_id")]
    pub illust_id: u64,
    #[serde(rename = "last_comment_id")]
    pub last_comment_id: Option<u64>,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct UserQuery {
    #[serde(rename = "user_id")]
    pub user_id: u64,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct UserIllustQuery {
    #[serde(rename = "user_id")]
    pub user_id: u64,
    #[serde(rename = "type")]
    pub illust_type: Option<IllustType>,
    pub filter: Option<FilterType>,
    pub offset: Option<i32>,
}

#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct V2Filter {
    #[serde(rename = "user_id")]
    pub user_id: Option<u64>,
    pub offset: Option<i32>,
    pub limit: Option<i32>,
    pub restrict: Option<Restrict>,
    pub mode: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct FollowUserRequest {
    #[serde(rename = "user_id")]
    pub user_id: u64,
    pub restrict: Restrict,
}

impl FollowUserRequest {
    pub fn new(user_id: u64) -> Self {
        Self {
            user_id,
            restrict: Restrict::Public,
        }
    }
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct AddBookmark {
    #[serde(rename = "illust_id")]
    pub illust_id: u64,
    pub restrict: Restrict,
}

impl AddBookmark {
    pub fn new(illust_id: u64) -> Self {
        Self {
            illust_id,
            restrict: Restrict::Public,
        }
    }
}

#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct AddBookmarkResult {
    #[serde(rename = "illust_id")]
    pub illust_id: Option<u64>,
}

#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct DeleteBookmarkResult {}

// ── Auth ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub enum GrantType {
    #[serde(rename = "refresh_token")]
    RefreshToken,
    #[serde(rename = "authorization_code")]
    AuthorizationCode,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct AuthResult {
    #[serde(rename = "access_token")]
    pub access_token: String,
    #[serde(rename = "expires_in")]
    pub expires_in: i64,
    #[serde(rename = "token_type")]
    pub token_type: Option<String>,
    pub scope: Option<String>,
    #[serde(rename = "refresh_token")]
    pub refresh_token: String,
    pub user: Option<User>,
    #[serde(rename = "device_token")]
    pub device_token: Option<String>,
}

// ── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_mode_serializes_lowercase_for_pixiv() {
        // Pixiv's ranking API accepts lowercase mode values; the enum must
        // serialize to them (this was the leaderboard "empty gallery" bug —
        // `mode=Week` was rejected with 400).
        assert_eq!(serde_json::to_string(&FilterMode::Day).unwrap(), r#""day""#);
        assert_eq!(
            serde_json::to_string(&FilterMode::Week).unwrap(),
            r#""week""#
        );
        assert_eq!(
            serde_json::to_string(&FilterMode::DayMale).unwrap(),
            r#""day_male""#
        );
        assert_eq!(
            serde_json::to_string(&FilterMode::WeekR18).unwrap(),
            r#""week_r18""#
        );
    }

    #[test]
    fn deserialize_minimal_illustration() {
        let json = r#"{"id": 12345, "title": "test", "type": "illust", "tags": [], "tools": [], "meta_pages": []}"#;
        let illust: Illustration = serde_json::from_str(json).unwrap();
        assert_eq!(illust.id, 12345);
        assert_eq!(illust.title.as_deref(), Some("test"));
        assert!(illust.tags.is_empty());
    }

    #[test]
    fn illustration_helper_methods() {
        let illust = Illustration {
            id: 1,
            title: None,
            illust_type: None,
            image_urls: None,
            caption: None,
            restrict: None,
            user: None,
            tags: vec![],
            tools: vec![],
            create_date: None,
            page_count: None,
            width: Some(800),
            height: Some(600),
            sanity_level: None,
            x_restrict: Some(1),
            series: None,
            meta_single_page: None,
            meta_pages: vec![],
            total_view: Some(100),
            total_bookmarks: Some(50),
            is_bookmarked: Some(true),
            visible: Some(true),
            is_muted: None,
            illust_ai_type: Some(2),
            provider_data: None,
        };
        assert!(illust.is_r18());
        assert!(illust.is_ai());
        assert!(illust.bookmarked());
        assert!(illust.is_landscape());
        assert!(!illust.is_restricted());
    }

    #[test]
    fn deserialize_ranked_illusts() {
        let json = r#"{"illusts": [], "next_url": null}"#;
        let ranked: RankedIllusts = serde_json::from_str(json).unwrap();
        assert!(ranked.illusts.is_empty());
        assert!(ranked.next_url.is_none());
    }

    #[test]
    fn deserialize_auth_result() {
        let json = r#"{"access_token":"tok","expires_in":3600,"refresh_token":"ref"}"#;
        let auth: AuthResult = serde_json::from_str(json).unwrap();
        assert_eq!(auth.access_token, "tok");
        assert_eq!(auth.expires_in, 3600);
        assert_eq!(auth.refresh_token, "ref");
    }

    #[test]
    fn serialize_filter_roundtrip() {
        let filter = SearchedIllustsFilter {
            word: "test".into(),
            sort: Some(SortOption::DateDesc),
            search_target: Some(SearchTarget::PartialMatchForTags),
            ..Default::default()
        };
        let json = serde_json::to_string(&filter).unwrap();
        let parsed: SearchedIllustsFilter = serde_json::from_str(&json).unwrap();
        assert_eq!(filter.word, parsed.word);
        assert_eq!(filter.sort, parsed.sort);
    }

    #[test]
    fn enum_serde() {
        let json = "\"illust\"";
        let t: IllustType = serde_json::from_str(json).unwrap();
        assert_eq!(t, IllustType::Illust);
        assert_eq!(serde_json::to_string(&t).unwrap(), "\"illust\"");
    }

    #[test]
    fn restrict_serializes_lowercase() {
        // Pixiv's bookmark endpoint expects "public"/"private" (lowercase);
        // the previous PascalCase form made bookmarking fail with a 400.
        assert_eq!(
            serde_json::to_string(&Restrict::Public).unwrap(),
            "\"public\""
        );
        assert_eq!(
            serde_json::to_string(&Restrict::Private).unwrap(),
            "\"private\""
        );
        assert_eq!(
            serde_json::from_str::<Restrict>("\"public\"").unwrap(),
            Restrict::Public
        );
        let add = AddBookmark::new(42);
        let params_json = serde_json::to_string(&add).unwrap();
        assert!(
            params_json.contains("\"restrict\":\"public\""),
            "{}",
            params_json
        );
    }
}

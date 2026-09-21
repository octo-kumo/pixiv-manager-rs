//! X's cookie-authenticated GraphQL endpoint metadata and JSON navigation.
//!
//! Query ids are extracted from the live web bundle and rotate every few
//! months; when they go stale the API answers 404. Rather than requiring a
//! rebuild, all of this is a [`GraphQlConfig`]: the constants below are only
//! the compiled-in **defaults**, and a `twitter_graphql.json` override file
//! (written by the settings "Update query ids" button, or dropped in by hand)
//! replaces individual operations at runtime. The public web bearer token is
//! a well-known constant (it ships in every browser bundle) — it is not a
//! secret.

use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;

/// The public web-app bearer token (same value for every X web session).
pub const BEARER_TOKEN: &str = "AAAAAAAAAAAAAAAAAAAAANRILgAAAAAAnNwIzUejRCOuH5E6I8xnZz4puTs%3D1Zv7ttfk8LF81IUq16cHjhLTvJu4FA33AGWWjCpTnA";

/// `HomeTimeline` — the algorithmic "For You" feed.
pub const HOME_TIMELINE_QUERY_ID: &str = "og4a4SdSF3WiQkkwaPCdPg";
/// `HomeLatestTimeline` — the chronological "Following" feed.
pub const HOME_LATEST_TIMELINE_QUERY_ID: &str = "OQPHTgwczzp9RMAPt6BH9A";
/// `Likes` — the account's liked tweets.
pub const LIKES_QUERY_ID: &str = "XHn_Tw60c6pi0n3DGhpwiA";
/// `TweetDetail` — a tweet's conversation (replies).
pub const TWEET_DETAIL_QUERY_ID: &str = "zoF7_t363wZyzylk-BLfZQ";
/// `TweetResultByRestId` — a single tweet by id.
pub const TWEET_RESULT_QUERY_ID: &str = "Xl0tsHf4AzflMRjbw9e70A";
/// `FavoriteTweet` — like a tweet.
pub const FAVORITE_TWEET_QUERY_ID: &str = "lI07N6Otwv1PhnEgXILM7A";
/// `UnfavoriteTweet` — unlike a tweet.
pub const UNFAVORITE_TWEET_QUERY_ID: &str = "ZYKSe-w7KEslx3JhSIk5LA";
/// `Viewer` — the logged-in account (validates the cookies + resolves handle).
pub const VIEWER_QUERY_ID: &str = "9t128XgFic52jPUEkJMf6w";
/// `UserTweets` — a user's own tweets timeline (the author page).
pub const USER_TWEETS_QUERY_ID: &str = "jeAA-59Y9FL7FmjgBNIVPw";

/// The feature-flag set the web client sends for timeline/tweet queries.
pub const FEATURES: &str = concat!(
    "{",
    "\"rweb_video_screen_enabled\":false,",
    "\"payments_enabled\":false,",
    "\"profile_label_improvements_pcf_label_in_post_enabled\":true,",
    "\"rweb_tipjar_consumption_enabled\":true,",
    "\"verified_phone_label_enabled\":false,",
    "\"creator_subscriptions_tweet_preview_api_enabled\":true,",
    "\"responsive_web_graphql_timeline_navigation_enabled\":true,",
    "\"responsive_web_graphql_skip_user_profile_image_extensions_enabled\":false,",
    "\"premium_content_api_read_enabled\":false,",
    "\"communities_web_enable_tweet_community_results_fetch\":true,",
    "\"c9s_tweet_anatomy_moderator_badge_enabled\":true,",
    "\"responsive_web_grok_analyze_button_fetch_trends_enabled\":false,",
    "\"responsive_web_grok_analyze_post_followups_enabled\":true,",
    "\"responsive_web_jetfuel_frame\":false,",
    "\"responsive_web_grok_share_attachment_enabled\":true,",
    "\"responsive_web_grok_annotations_enabled\":true,",
    "\"articles_preview_enabled\":true,",
    "\"responsive_web_edit_tweet_api_enabled\":true,",
    "\"graphql_is_translatable_rweb_tweet_is_translatable_enabled\":true,",
    "\"view_counts_everywhere_api_enabled\":true,",
    "\"longform_notetweets_consumption_enabled\":true,",
    "\"responsive_web_twitter_article_tweet_consumption_enabled\":true,",
    "\"tweet_awards_web_tipping_enabled\":false,",
    "\"responsive_web_grok_show_grok_translated_post\":false,",
    "\"responsive_web_grok_analysis_button_from_backend\":true,",
    "\"creator_subscriptions_quote_tweet_preview_enabled\":false,",
    "\"freedom_of_speech_not_reach_fetch_enabled\":true,",
    "\"standardized_nudges_misinfo\":true,",
    "\"tweet_with_visibility_results_prefer_gql_limited_actions_policy_enabled\":true,",
    "\"longform_notetweets_rich_text_read_enabled\":true,",
    "\"longform_notetweets_inline_media_enabled\":true,",
    "\"responsive_web_grok_image_annotation_enabled\":true,",
    "\"responsive_web_grok_community_note_auto_translation_is_enabled\":false,",
    "\"responsive_web_enhance_cards_enabled\":false",
    "}"
);

/// The compiled-in operation → query id table.
pub const DEFAULT_QUERIES: &[(&str, &str)] = &[
    ("HomeTimeline", HOME_TIMELINE_QUERY_ID),
    ("HomeLatestTimeline", HOME_LATEST_TIMELINE_QUERY_ID),
    ("Likes", LIKES_QUERY_ID),
    ("TweetDetail", TWEET_DETAIL_QUERY_ID),
    ("TweetResultByRestId", TWEET_RESULT_QUERY_ID),
    ("FavoriteTweet", FAVORITE_TWEET_QUERY_ID),
    ("UnfavoriteTweet", UNFAVORITE_TWEET_QUERY_ID),
    ("Viewer", VIEWER_QUERY_ID),
    ("UserTweets", USER_TWEETS_QUERY_ID),
];

/// The API host used by the web client.
pub const API_BASE: &str = "https://api.x.com";

/// Runtime GraphQL metadata: bearer token, feature flags and the
/// operation → query id table.
///
/// [`Self::compiled`] returns the built-in defaults; [`Self::load`] applies a
/// JSON override file on top of them (per-operation, so a refresh only needs
/// to list the operations that changed). Keeping this separate from the
/// client is what makes query-id rotation a data update instead of a rebuild.
#[derive(Debug, Clone)]
pub struct GraphQlConfig {
    /// Bearer token sent with every request.
    pub bearer_token: String,
    /// Serialized JSON object sent as the `features` query parameter.
    pub features: String,
    /// Operation name (`HomeTimeline`, …) → query id.
    pub queries: HashMap<String, String>,
}

impl Default for GraphQlConfig {
    fn default() -> Self {
        Self::compiled()
    }
}

/// The override file's schema. Every field is optional so a refresh script
/// can write only the query ids it extracted.
#[derive(Debug, Default, Deserialize)]
struct OverrideFile {
    #[serde(default)]
    bearer_token: Option<String>,
    /// Accepted as a JSON object (preferred) or as an already-serialized
    /// string.
    #[serde(default)]
    features: Option<Value>,
    #[serde(default)]
    queries: HashMap<String, String>,
}

impl GraphQlConfig {
    /// The baked-in defaults.
    pub fn compiled() -> Self {
        Self {
            bearer_token: BEARER_TOKEN.to_string(),
            features: FEATURES.to_string(),
            queries: DEFAULT_QUERIES
                .iter()
                .map(|(op, id)| ((*op).to_string(), (*id).to_string()))
                .collect(),
        }
    }

    /// Parse an override document (`twitter_graphql.json`) and merge it over
    /// the compiled defaults. Missing fields keep their default; blank
    /// values are ignored so a partially written file can't break auth.
    pub fn from_json(text: &str) -> Result<Self, serde_json::Error> {
        let file: OverrideFile = serde_json::from_str(text)?;
        Ok(Self::compiled().applied(file))
    }

    /// Load an override file from disk.
    pub fn load(path: &Path) -> Result<Self, crate::twitter::TwitterError> {
        let text = std::fs::read_to_string(path).map_err(|e| {
            crate::twitter::TwitterError::Internal(format!("read {}: {e}", path.display()))
        })?;
        Self::from_json(&text).map_err(|e| {
            crate::twitter::TwitterError::Internal(format!("parse {}: {e}", path.display()))
        })
    }

    fn applied(mut self, file: OverrideFile) -> Self {
        if let Some(token) = file.bearer_token.filter(|t| !t.trim().is_empty()) {
            self.bearer_token = token;
        }
        if let Some(features) = file.features {
            self.features = match features {
                Value::String(s) => s,
                other => other.to_string(),
            };
        }
        for (operation, id) in file.queries {
            if !id.trim().is_empty() {
                self.queries.insert(operation, id);
            }
        }
        self
    }

    /// Resolve an operation's query id.
    pub fn query_id(&self, operation: &str) -> Option<&str> {
        self.queries.get(operation).map(String::as_str)
    }

    /// Persist the query-id table as an override file (atomic temp + rename)
    /// so a later client reload/restart picks it up. Only operations that
    /// differ from the compiled defaults are written, keeping the file a
    /// minimal diff that new app versions can extend.
    pub fn save_queries(&self, path: &Path) -> std::io::Result<()> {
        #[derive(serde::Serialize)]
        struct File<'a> {
            queries: std::collections::BTreeMap<&'a str, &'a str>,
        }
        let defaults = Self::compiled();
        let queries = self
            .queries
            .iter()
            .filter(|(operation, id)| defaults.query_id(operation) != Some(id.as_str()))
            .map(|(operation, id)| (operation.as_str(), id.as_str()))
            .collect();
        crate::io::save_json_atomic(&File { queries }, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compiled_has_every_operation_the_client_calls() {
        let config = GraphQlConfig::compiled();
        for operation in [
            "HomeTimeline",
            "HomeLatestTimeline",
            "Likes",
            "TweetDetail",
            "TweetResultByRestId",
            "FavoriteTweet",
            "UnfavoriteTweet",
            "Viewer",
            "UserTweets",
        ] {
            assert!(config.query_id(operation).is_some(), "missing {operation}");
        }
    }

    #[test]
    fn override_merges_per_operation() {
        let json = r#"{"queries":{"HomeTimeline":"NEW_ID"}}"#;
        let config = GraphQlConfig::from_json(json).expect("parse");
        assert_eq!(config.query_id("HomeTimeline"), Some("NEW_ID"));
        // Untouched operations keep their compiled defaults.
        assert_eq!(
            config.query_id("HomeLatestTimeline"),
            Some(HOME_LATEST_TIMELINE_QUERY_ID)
        );
        assert_eq!(config.bearer_token, BEARER_TOKEN);
        assert_eq!(config.features, FEATURES);
    }

    #[test]
    fn override_accepts_features_object_or_string() {
        let as_object = GraphQlConfig::from_json(r#"{"features":{"a":true}}"#).expect("parse");
        assert_eq!(as_object.features, r#"{"a":true}"#);

        let as_string = GraphQlConfig::from_json(r#"{"features":"{\"a\":true}"}"#).expect("parse");
        assert_eq!(as_string.features, r#"{"a":true}"#);
    }

    #[test]
    fn override_ignores_blank_values() {
        let json = r#"{"bearer_token":"  ","queries":{"HomeTimeline":""}}"#;
        let config = GraphQlConfig::from_json(json).expect("parse");
        assert_eq!(config.bearer_token, BEARER_TOKEN);
        assert_eq!(
            config.query_id("HomeTimeline"),
            Some(HOME_TIMELINE_QUERY_ID)
        );
    }

    #[test]
    fn override_file_round_trips_through_disk() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("twitter_graphql.json");
        std::fs::write(&path, r#"{"queries":{"Viewer":"XYZ"}}"#).unwrap();
        let config = GraphQlConfig::load(&path).expect("load");
        assert_eq!(config.query_id("Viewer"), Some("XYZ"));
        assert_eq!(config.query_id("Likes"), Some(LIKES_QUERY_ID));
    }

    #[test]
    fn malformed_override_file_is_an_error() {
        assert!(GraphQlConfig::from_json("{not json").is_err());
    }

    #[test]
    fn save_queries_writes_only_the_diff_and_reloads() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("twitter_graphql.json");

        let mut config = GraphQlConfig::compiled();
        config.queries.insert("HomeTimeline".into(), "NEW".into());
        config.save_queries(&path).expect("save");
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains("HomeTimeline"), "{raw}");
        assert!(!raw.contains("Likes"), "unchanged defaults stay out: {raw}");

        let loaded = GraphQlConfig::load(&path).expect("load");
        assert_eq!(loaded.query_id("HomeTimeline"), Some("NEW"));
        assert_eq!(loaded.query_id("Likes"), Some(LIKES_QUERY_ID));

        // Saving a fully-default config writes an empty override.
        GraphQlConfig::compiled().save_queries(&path).expect("save");
        let loaded = GraphQlConfig::load(&path).expect("load");
        assert_eq!(loaded.query_id("Viewer"), Some(VIEWER_QUERY_ID));
    }
}

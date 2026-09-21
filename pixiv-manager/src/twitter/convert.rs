//! Mapping X GraphQL tweet JSON onto the app's universal [`Illustration`]
//! shape, plus timeline navigation helpers.
//!
//! The GraphQL payloads are deeply nested and drift over time, so the
//! navigation is value-based with generous fallbacks instead of rigid serde
//! structs; only what the UI needs is extracted. Tweets with no photo media
//! are rejected (`None`) — the gallery is images-only by design.

use crate::media::twitter_key;
use pixivj_rs::model::*;
use serde_json::Value;

/// Entries of the first `TimelineAddEntries` instruction in a home timeline
/// response (`data.home.home_timeline_urt.instructions`).
pub fn home_entries(root: &Value) -> Vec<&Value> {
    entries_at(root, "/data/home/home_timeline_urt/instructions")
}

/// Entries of a likes timeline response
/// (`data.user.result.timeline.timeline.instructions`).
pub fn likes_entries(root: &Value) -> Vec<&Value> {
    entries_at(root, "/data/user/result/timeline/timeline/instructions")
}

/// Entries of a user-tweets timeline (the author page). Newer responses put
/// it under `timeline_v2`, older ones under `timeline`.
pub fn user_tweets_entries(root: &Value) -> Vec<&Value> {
    let v2 = entries_at(root, "/data/user/result/timeline_v2/timeline/instructions");
    if v2.is_empty() {
        entries_at(root, "/data/user/result/timeline/timeline/instructions")
    } else {
        v2
    }
}

/// Entries of a tweet-detail response
/// (`data.threaded_conversation_with_injections_v2.instructions`).
pub fn detail_entries(root: &Value) -> Vec<&Value> {
    entries_at(
        root,
        "/data/threaded_conversation_with_injections_v2/instructions",
    )
}

fn entries_at<'a>(root: &'a Value, instructions_path: &str) -> Vec<&'a Value> {
    let Some(instructions) = root.pointer(instructions_path).and_then(Value::as_array) else {
        return Vec::new();
    };
    instructions
        .iter()
        .filter(|i| i.get("type").and_then(Value::as_str) == Some("TimelineAddEntries"))
        .filter_map(|i| i.get("entries").and_then(Value::as_array))
        .flatten()
        .collect()
}

/// The bottom pagination cursor of a timeline, if any.
pub fn next_cursor(entries: &[&Value]) -> Option<String> {
    entries.iter().find_map(|e| {
        let content = e.get("content")?;
        if content.get("cursorType").and_then(Value::as_str) != Some("Bottom") {
            return None;
        }
        content
            .get("value")
            .and_then(Value::as_str)
            .map(str::to_string)
    })
}

/// Every tweet JSON carried by a timeline entry: a single item or the items
/// of a conversation module. `TweetWithVisibilityResults` wrappers are
/// unwrapped, tombstones and non-tweets are skipped, and retweets resolve to
/// the retweeted original (so media + like state are the original's).
pub fn entry_tweets(entry: &Value) -> Vec<&Value> {
    let Some(content) = entry.get("content") else {
        return Vec::new();
    };
    // A plain tweet item.
    if let Some(result) = content.pointer("/itemContent/tweet_results/result") {
        return unwrap_tweet(result).into_iter().collect();
    }
    // A conversation module: `items[].item.itemContent.tweet_results.result`.
    if let Some(items) = content.get("items").and_then(Value::as_array) {
        return items
            .iter()
            .filter_map(|item| item.pointer("/item/itemContent/tweet_results/result"))
            .filter_map(unwrap_tweet)
            .collect();
    }
    Vec::new()
}

/// Unwrap `TweetWithVisibilityResults` and resolve retweets to the original.
fn unwrap_tweet(result: &Value) -> Option<&Value> {
    let typename = result.get("__typename").and_then(Value::as_str);
    if typename == Some("TweetTombstone") {
        return None;
    }
    let tweet = if typename == Some("TweetWithVisibilityResults") {
        result.get("tweet")?
    } else {
        result
    };
    if tweet
        .pointer("/legacy/retweeted_status_result/result")
        .is_some()
    {
        return unwrap_tweet(tweet.pointer("/legacy/retweeted_status_result/result")?);
    }
    tweet.get("legacy").map(|_| tweet)
}

/// The tweet's display text (`note_tweet` for long posts, `full_text` else).
pub fn tweet_text(tweet: &Value) -> String {
    if let Some(text) = tweet
        .pointer("/note_tweet/note_tweet_results/result/text")
        .and_then(Value::as_str)
    {
        return text.to_string();
    }
    tweet
        .pointer("/legacy/full_text")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// Whether the tweet has at least one photo attachment.
pub fn has_photo(tweet: &Value) -> bool {
    !photo_media(tweet).is_empty()
}

fn photo_media(tweet: &Value) -> Vec<&Value> {
    tweet
        .pointer("/legacy/extended_entities/media")
        .and_then(Value::as_array)
        .map(|media| {
            media
                .iter()
                .filter(|m| m.get("type").and_then(Value::as_str) == Some("photo"))
                .collect()
        })
        .unwrap_or_default()
}

/// Append a twimg size variant (`?name=orig`) to a **media** URL.
fn with_name(url: &str, name: &str) -> String {
    let sep = if url.contains('?') { '&' } else { '?' };
    format!("{url}{sep}name={name}")
}

/// Pick a profile-image variant. Profile image URLs live on a different
/// twimg path (`/profile_images/`) than media (`/media/`) and do NOT accept
/// the `?name=` query — twimg answers every variant query with a 404. Their
/// size is instead a path suffix (`_normal` 48px, `_bigger` 73px,
/// `_200x200`, `_400x400`), so upgrade `_normal` to `_200x200` (sharp for
/// both the 24px comment avatars and the ~44px header at 2× DPI) and leave
/// everything else untouched.
fn profile_image_url(url: &str) -> String {
    if url.contains("_normal.") {
        url.replace("_normal.", "_200x200.")
    } else {
        url.to_string()
    }
}

/// Twitter's `created_at` (`"Sat Dec 20 10:04:05 +0000 2025"`); some locales
/// emit a space-padded day, so collapse doubled spaces before parsing.
fn parse_date(raw: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    let normalized = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    chrono::DateTime::parse_from_str(&normalized, "%a %b %d %H:%M:%S %z %Y")
        .ok()
        .map(|d| d.with_timezone(&chrono::Utc))
}

/// The tweet author as the app's [`Author`] (ids namespaced like tweets).
fn tweet_author(tweet: &Value) -> Option<Author> {
    let result = tweet.pointer("/core/user_results/result")?;
    let id = result
        .get("rest_id")
        .and_then(Value::as_str)
        .and_then(|s| s.parse::<u64>().ok())
        .or_else(|| result.get("id").and_then(Value::as_str)?.parse().ok())?;
    let name = result
        .pointer("/core/name")
        .or_else(|| result.pointer("/legacy/name"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let account = result
        .pointer("/core/screen_name")
        .or_else(|| result.pointer("/legacy/screen_name"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let avatar = result
        .pointer("/avatar/image_url")
        .or_else(|| result.pointer("/legacy/profile_image_url_https"))
        .and_then(Value::as_str)
        .map(|url| AuthorProfileImageUrls {
            medium: Some(profile_image_url(url)),
        });
    Some(Author {
        id: twitter_key(id),
        profile_image_urls: avatar,
        name,
        account,
        comment: None,
        is_followed: None,
    })
}

/// Convert a raw tweet JSON node into the universal [`Illustration`].
/// Returns `None` for tweets without photo media.
pub fn tweet_to_illust(tweet: &Value) -> Option<Illustration> {
    let legacy = tweet.get("legacy")?;
    let native_id: u64 = legacy
        .get("id_str")
        .and_then(Value::as_str)
        .and_then(|s| s.parse().ok())?;
    let photos = photo_media(tweet);
    if photos.is_empty() {
        return None;
    }
    let user = tweet_author(tweet);
    let text = tweet_text(tweet);
    let title = first_line(&text);
    let create_date = legacy
        .get("created_at")
        .and_then(Value::as_str)
        .and_then(parse_date);

    // Every photo becomes an original-quality page; the first also becomes
    // the single-page source. Thumbnails use the 1200px `large` variant.
    let mut meta_pages: Vec<MetaPage> = Vec::with_capacity(photos.len());
    let mut labels: Vec<IllustImageUrls> = Vec::with_capacity(photos.len());
    for photo in &photos {
        let Some(base) = photo.get("media_url_https").and_then(Value::as_str) else {
            continue;
        };
        labels.push(IllustImageUrls {
            square_medium: Some(with_name(base, "small")),
            medium: Some(with_name(base, "medium")),
            large: Some(with_name(base, "large")),
        });
        meta_pages.push(MetaPage {
            image_urls: MetaPageImageUrls {
                square_medium: Some(with_name(base, "small")),
                medium: Some(with_name(base, "medium")),
                large: Some(with_name(base, "large")),
                original: Some(with_name(base, "orig")),
            },
        });
    }
    if meta_pages.is_empty() {
        return None;
    }
    let first = &labels[0];
    let (width, height) = photos
        .first()
        .and_then(|p| p.get("original_info"))
        .map(|info| {
            (
                info.get("width").and_then(Value::as_i64).map(|v| v as i32),
                info.get("height").and_then(Value::as_i64).map(|v| v as i32),
            )
        })
        .unwrap_or((None, None));

    let tags: Vec<Tag> = legacy
        .pointer("/entities/hashtags")
        .and_then(Value::as_array)
        .map(|hashtags| {
            hashtags
                .iter()
                .filter_map(|h| h.get("text").and_then(Value::as_str))
                .map(|text| Tag {
                    name: text.to_string(),
                    translated_name: None,
                })
                .collect()
        })
        .unwrap_or_default();

    let total_bookmarks = legacy.get("favorite_count").and_then(Value::as_u64);
    let total_view = tweet
        .pointer("/views/count")
        .and_then(Value::as_str)
        .and_then(|s| s.parse::<u64>().ok());
    let possibly_sensitive = legacy
        .get("possibly_sensitive")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    Some(Illustration {
        id: twitter_key(native_id),
        title: Some(if title.is_empty() {
            user.as_ref()
                .and_then(|u| u.account.clone())
                .map(|a| format!("@{a}"))
                .unwrap_or_else(|| format!("tweet {native_id}"))
        } else {
            title
        }),
        illust_type: None,
        image_urls: Some(first.clone()),
        caption: if text.is_empty() { None } else { Some(text) },
        restrict: None,
        user,
        tags,
        tools: Vec::new(),
        create_date,
        page_count: Some(meta_pages.len() as i32),
        width,
        height,
        sanity_level: None,
        x_restrict: Some(if possibly_sensitive { 1 } else { 0 }),
        series: None,
        meta_single_page: None,
        meta_pages,
        total_view,
        total_bookmarks,
        // X calls it "favorite"; the app's bookmark state is the like state.
        is_bookmarked: legacy.get("favorited").and_then(Value::as_bool),
        visible: Some(true),
        is_muted: None,
        illust_ai_type: None,
        provider_data: None,
    })
}

/// A reply as the app's [`Comment`] (used by the detail view's comment panel).
pub fn tweet_to_comment(tweet: &Value) -> Option<Comment> {
    let legacy = tweet.get("legacy")?;
    let native_id: u64 = legacy
        .get("id_str")
        .and_then(Value::as_str)
        .and_then(|s| s.parse().ok())?;
    Some(Comment {
        id: twitter_key(native_id),
        comment: tweet_text(tweet),
        date: legacy
            .get("created_at")
            .and_then(Value::as_str)
            .and_then(parse_date),
        has_replies: None,
        user: tweet_author(tweet),
        stamp: None,
    })
}

/// The tweet's public URL (`https://x.com/{handle}/status/{id}`), when the
/// author handle is known.
pub fn tweet_web_url(illust: &Illustration) -> Option<String> {
    let handle = illust.user.as_ref()?.account.as_deref()?;
    Some(format!(
        "https://x.com/{}/status/{}",
        handle,
        crate::media::native_id(illust.id)
    ))
}

/// First non-empty line, trimmed and capped (used as the tile/detail title).
fn first_line(text: &str) -> String {
    const MAX: usize = 120;
    let line = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    let line = line.trim();
    if line.chars().count() <= MAX {
        return line.to_string();
    }
    let mut out: String = line.chars().take(MAX).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A realistic tweet node (trimmed to the fields the app reads).
    fn sample_tweet() -> Value {
        json!({
            "__typename": "Tweet",
            "rest_id": "2002319084739051763",
            "legacy": {
                "id_str": "2002319084739051763",
                "full_text": "first line\nsecond line #cat #art",
                "created_at": "Sat Dec 20 10:04:05 +0000 2025",
                "favorite_count": 14249,
                "favorited": true,
                "possibly_sensitive": false,
                "entities": { "hashtags": [
                    {"text": "cat"}, {"text": "art"}
                ]},
                "extended_entities": { "media": [
                    {
                        "type": "photo",
                        "media_url_https": "https://pbs.twimg.com/media/G8mqlGnbgAACnD9.jpg",
                        "original_info": {"width": 1200, "height": 1600}
                    }
                ]}
            },
            "views": {"count": "123456"},
            "core": { "user_results": { "result": {
                "rest_id": "12345",
                "core": {"name": "Artist", "screen_name": "artist_x"},
                "avatar": {"image_url": "https://pbs.twimg.com/profile_images/1/a_normal.jpg"}
            }}}
        })
    }

    #[test]
    fn maps_tweet_to_illustration() {
        let illust = tweet_to_illust(&sample_tweet()).expect("has photo");
        assert_eq!(illust.id, twitter_key(2002319084739051763));
        assert_eq!(illust.title(), "first line");
        assert_eq!(
            illust.caption.as_deref(),
            Some("first line\nsecond line #cat #art")
        );
        assert_eq!(illust.page_count(), 1);
        assert_eq!(illust.total_bookmarks, Some(14249));
        assert_eq!(illust.total_view, Some(123456));
        assert!(illust.bookmarked());
        assert_eq!(
            illust.user.as_ref().unwrap().account.as_deref(),
            Some("artist_x")
        );
        assert_eq!(illust.user.as_ref().unwrap().id, twitter_key(12345));
        assert_eq!(
            illust.meta_pages[0].image_urls.original.as_deref(),
            Some("https://pbs.twimg.com/media/G8mqlGnbgAACnD9.jpg?name=orig")
        );
        assert_eq!(illust.tags.len(), 2);
        assert_eq!(illust.width, Some(1200));
        assert_eq!(
            tweet_web_url(&illust).as_deref(),
            Some("https://x.com/artist_x/status/2002319084739051763")
        );
    }

    #[test]
    fn rejects_text_only_and_unwraps_visibility_and_retweets() {
        let mut text_only = sample_tweet();
        text_only["legacy"]["extended_entities"] = json!({"media": []});
        assert!(tweet_to_illust(&text_only).is_none());

        // Retweets resolve to the original tweet's media + id.
        let original = sample_tweet();
        let retweet = json!({
            "__typename": "Tweet",
            "legacy": {
                "id_str": "999",
                "retweeted_status_result": {"result": original}
            }
        });
        let illust = tweet_to_illust(unwrap_tweet(&retweet).unwrap()).unwrap();
        assert_eq!(illust.id, twitter_key(2002319084739051763));

        // Visibility wrappers unwrap too.
        let wrapped = json!({"__typename": "TweetWithVisibilityResults", "tweet": sample_tweet()});
        assert!(tweet_to_illust(unwrap_tweet(&wrapped).unwrap()).is_some());
    }

    #[test]
    fn profile_images_never_get_a_name_variant() {
        // Regression: `?name=x96` 404s on /profile_images/ URLs (only
        // /media/ URLs accept the `?name=` query). The avatar is upgraded
        // via its PATH suffix instead.
        let author = tweet_author(&sample_tweet()).expect("author");
        let url = author
            .profile_image_urls
            .as_ref()
            .and_then(|p| p.medium.clone())
            .expect("avatar url");
        assert!(
            !url.contains("?"),
            "profile image must not carry ?name=: {url}"
        );
        assert!(
            url.contains("_200x200."),
            "_normal is upgraded to the sharper _200x200 variant: {url}"
        );
        // A URL that is already sized stays untouched.
        assert_eq!(
            profile_image_url("https://pbs.twimg.com/profile_images/1/a_400x400.jpg"),
            "https://pbs.twimg.com/profile_images/1/a_400x400.jpg"
        );
        assert_eq!(
            profile_image_url("https://pbs.twimg.com/profile_images/1/a_normal.jpg"),
            "https://pbs.twimg.com/profile_images/1/a_200x200.jpg"
        );
    }

    #[test]
    fn cursor_and_entry_extraction() {
        let root = json!({"data": {"user": {"result": {"timeline": {"timeline": {"instructions": [
            {"type": "TimelineAddEntries", "entries": [
                {"content": {"entryType": "TimelineTimelineItem", "itemContent": {"tweet_results": {"result": sample_tweet()}}}},
                {"content": {"entryType": "TimelineTimelineCursor", "cursorType": "Bottom", "value": "CURSOR"}},
                {"content": {"entryType": "TimelineTimelineCursor", "cursorType": "Top", "value": "TOP"}}
            ]}
        ]}}}}}});
        let entries = likes_entries(&root);
        assert_eq!(entries.len(), 3);
        assert_eq!(next_cursor(&entries).as_deref(), Some("CURSOR"));
        assert_eq!(entry_tweets(entries[0]).len(), 1);
    }

    #[test]
    fn space_padded_dates_parse() {
        assert!(parse_date("Sat Dec  6 10:04:05 +0000 2025").is_some());
    }
}

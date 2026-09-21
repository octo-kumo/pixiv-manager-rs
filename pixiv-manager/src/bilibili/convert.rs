//! Mapping Bilibili dynamic JSON onto the app's universal [`Illustration`]
//! shape, plus reply navigation helpers.
//!
//! Only posts that carry images are converted (draw posts and opus posts with
//! pictures); videos, text-only posts, live cards and forwards without media
//! are rejected with `None`. Forwards resolve to the original post, so a
//! reposted image set appears once, attributed to its author.

use crate::media::bilibili_key;
use pixivj_rs::model::*;
use serde_json::Value;

/// One image in a dynamic.
struct Photo {
    url: String,
    width: Option<i32>,
    height: Option<i32>,
}

/// Everything the UI needs extracted from a raw dynamic item.
pub fn dynamic_to_illust(item: &Value) -> Option<Illustration> {
    // A forward (repost) shows the original post's media; unwrap it so the
    // id, author and stats are the original's.
    if item.get("type").and_then(Value::as_str) == Some("DYNAMIC_TYPE_FORWARD") {
        if let Some(orig) = item.get("orig") {
            if !photos_of(orig).is_empty() {
                return dynamic_to_illust(orig);
            }
        }
    }

    let photos = photos_of(item);
    if photos.is_empty() {
        return None;
    }
    let native_id: u64 = parse_u64(item.get("id_str")?)?;
    let modules = item.get("modules")?;
    let dynamic = modules.get("module_dynamic").unwrap_or(&Value::Null);
    let author = modules.get("module_author");
    let text = text_of(dynamic, item);
    let user = author.and_then(author_of);
    let create_date = author
        .and_then(|a| a.get("pub_ts"))
        .and_then(parse_u64_any)
        .and_then(|ts| chrono::DateTime::from_timestamp(ts as i64, 0));

    let mut pages: Vec<MetaPage> = Vec::with_capacity(photos.len());
    let mut labels: Vec<IllustImageUrls> = Vec::with_capacity(photos.len());
    for photo in &photos {
        let base = to_https(&photo.url);
        labels.push(IllustImageUrls {
            square_medium: Some(with_size(&base, "360w_360h_1c.jpg")),
            medium: Some(with_size(&base, "480w_480h_1c.jpg")),
            large: Some(with_size(&base, "1280w_1280h_1c.jpg")),
        });
        pages.push(MetaPage {
            image_urls: MetaPageImageUrls {
                square_medium: Some(with_size(&base, "360w_360h_1c.jpg")),
                medium: Some(with_size(&base, "480w_480h_1c.jpg")),
                large: Some(with_size(&base, "1280w_1280h_1c.jpg")),
                // The original ("大图") — used for downloads and the viewer.
                original: Some(base),
            },
        });
    }
    let first = &labels[0];
    let (width, height) = photos
        .first()
        .map(|p| (p.width, p.height))
        .unwrap_or((None, None));
    let title = first_line(&text);

    let tags: Vec<Tag> = dynamic
        .pointer("/topic/name")
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty())
        .map(|name| {
            vec![Tag {
                name: name.to_string(),
                translated_name: None,
            }]
        })
        .unwrap_or_default();

    let stats = modules.get("module_stat");
    let like_count = stats
        .and_then(|s| s.pointer("/like/count"))
        .and_then(Value::as_u64);
    let liked_by_me = stats
        .and_then(|s| s.pointer("/like/status"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let comment_oid = item
        .pointer("/basic/comment_id_str")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            item.pointer("/basic/rid_str")
                .and_then(Value::as_str)
                .map(str::to_string)
        });

    Some(Illustration {
        id: bilibili_key(native_id),
        title: Some(if title.is_empty() {
            user.as_ref()
                .map(|u| u.name.clone())
                .unwrap_or_else(|| format!("dynamic {native_id}"))
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
        page_count: Some(pages.len() as i32),
        width,
        height,
        sanity_level: None,
        x_restrict: Some(0),
        series: None,
        meta_single_page: None,
        meta_pages: pages,
        total_view: None,
        // Bilibili "bookmarks" are likes; the authoritative local list is the
        // bookmarks tab, so `is_bookmarked` is filled in by the caller.
        total_bookmarks: like_count,
        // Carried for the caller to seed the local bookmark state from an
        // already-liked post.
        is_bookmarked: Some(liked_by_me),
        visible: Some(true),
        is_muted: None,
        illust_ai_type: None,
        provider_data: comment_oid.map(|oid| ProviderData {
            comment_oid: Some(oid),
        }),
    })
}

/// A reply as the app's [`Comment`].
pub fn reply_to_comment(reply: &Value) -> Option<Comment> {
    let native_id = parse_u64(reply.get("rpid")?)?;
    let member = reply.get("member");
    let user = member.map(|m| {
        let mid = m.get("mid").and_then(parse_u64_any).unwrap_or(0);
        Author {
            id: bilibili_key(mid),
            profile_image_urls: m.get("avatar").and_then(Value::as_str).map(|url| {
                AuthorProfileImageUrls {
                    medium: Some(to_https(url)),
                }
            }),
            name: m
                .get("uname")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            account: None,
            comment: None,
            is_followed: None,
        }
    });
    Some(Comment {
        id: bilibili_key(native_id),
        comment: reply
            .pointer("/content/message")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        date: reply
            .get("ctime")
            .and_then(parse_u64_any)
            .and_then(|ts| chrono::DateTime::from_timestamp(ts as i64, 0)),
        has_replies: Some(
            reply
                .get("rcount")
                .and_then(parse_u64_any)
                .map(|n| n > 0)
                .unwrap_or(false),
        ),
        user,
        stamp: None,
    })
}

/// The public web URL for a dynamic.
pub fn dynamic_web_url(illust: &Illustration) -> String {
    format!(
        "https://t.bilibili.com/{}",
        crate::media::native_id(illust.id)
    )
}

/// Every image of the dynamic (draw `items` or opus `pics`), normalizing the
/// field names of both shapes.
fn photos_of(item: &Value) -> Vec<Photo> {
    let dynamic = match item.pointer("/modules/module_dynamic") {
        Some(d) => d,
        None => return Vec::new(),
    };
    let major = match dynamic.get("major") {
        Some(m) => m,
        None => return Vec::new(),
    };
    match major.get("type").and_then(Value::as_str) {
        Some("MAJOR_TYPE_DRAW") => major
            .pointer("/draw/items")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|it| {
                        Some(Photo {
                            url: it.get("src").and_then(Value::as_str)?.to_string(),
                            width: it.get("width").and_then(parse_u64_any).map(|v| v as i32),
                            height: it.get("height").and_then(parse_u64_any).map(|v| v as i32),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default(),
        Some("MAJOR_TYPE_OPUS") => major
            .pointer("/opus/pics")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|it| {
                        Some(Photo {
                            url: it.get("url").and_then(Value::as_str)?.to_string(),
                            width: it.get("width").and_then(parse_u64_any).map(|v| v as i32),
                            height: it.get("height").and_then(parse_u64_any).map(|v| v as i32),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// The post text: the dynamic desc, or the opus summary/title.
fn text_of(dynamic: &Value, _item: &Value) -> String {
    if let Some(text) = dynamic
        .pointer("/desc/text")
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty())
    {
        return text.to_string();
    }
    if let Some(text) = dynamic
        .pointer("/major/opus/summary/text")
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty())
    {
        return text.to_string();
    }
    dynamic
        .pointer("/major/opus/title")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn author_of(author: &Value) -> Option<Author> {
    let mid = author.get("mid").and_then(parse_u64_any)?;
    Some(Author {
        id: bilibili_key(mid),
        profile_image_urls: author
            .get("face")
            .and_then(Value::as_str)
            .filter(|f| !f.is_empty())
            .map(|url| AuthorProfileImageUrls {
                medium: Some(to_https(url)),
            }),
        name: author
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        account: None,
        comment: None,
        is_followed: author.get("following").and_then(Value::as_bool),
    })
}

/// First non-empty line, trimmed and capped (used as the title).
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

/// `http://` → `https://` (Bilibili returns http image URLs).
fn to_https(url: &str) -> String {
    match url.strip_prefix("http://") {
        Some(rest) => format!("https://{rest}"),
        None => url.to_string(),
    }
}

/// Strip any existing `@…` processing suffix and append a new one.
fn with_size(url: &str, spec: &str) -> String {
    let base = url.split('@').next().unwrap_or(url);
    format!("{base}@{spec}")
}

/// Parse a numeric field that may be a number or a string.
fn parse_u64(value: &Value) -> Option<u64> {
    parse_u64_any(value)
}

fn parse_u64_any(value: &Value) -> Option<u64> {
    match value {
        Value::Number(n) => n.as_u64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample_dynamic() -> Value {
        json!({
            "id_str": "1205134739172753431",
            "type": "DYNAMIC_TYPE_DRAW",
            "basic": {
                "rid_str": "395446267",
                "comment_type": 11,
                "comment_id_str": "395446267"
            },
            "modules": {
                "module_author": {
                    "mid": 2105955,
                    "name": "桌子君",
                    "face": "http://i1.hdslb.com/bfs/face/abc.jpg",
                    "following": true,
                    "pub_ts": "1779430697"
                },
                "module_dynamic": {
                    "desc": { "text": "first line\nsecond line" },
                    "major": {
                        "type": "MAJOR_TYPE_DRAW",
                        "draw": { "id": 1, "items": [
                            {
                                "src": "http://i0.hdslb.com/bfs/new_dyn/a.jpg",
                                "width": "4000", "height": "3000"
                            },
                            {
                                "src": "http://i0.hdslb.com/bfs/new_dyn/b.png",
                                "width": "1000", "height": "1000"
                            }
                        ]}
                    },
                    "topic": { "id": 1, "name": "插画" }
                },
                "module_stat": {
                    "like": { "status": true, "count": 828 },
                    "comment": { "count": 33 },
                    "forward": { "count": 8 }
                }
            }
        })
    }

    #[test]
    fn maps_draw_dynamic() {
        let illust = dynamic_to_illust(&sample_dynamic()).expect("has photos");
        assert_eq!(illust.id, bilibili_key(1205134739172753431));
        assert!(
            illust.create_date.is_some(),
            "pub_ts is a string in the API and must parse"
        );
        assert_eq!(illust.title(), "first line");
        assert_eq!(illust.caption.as_deref(), Some("first line\nsecond line"));
        assert_eq!(illust.page_count(), 2);
        assert_eq!(illust.total_bookmarks, Some(828));
        assert_eq!(illust.user.as_ref().unwrap().id, bilibili_key(2105955));
        assert!(illust.user.as_ref().unwrap().followed());
        assert_eq!(illust.tags.len(), 1);
        assert_eq!(illust.tags[0].name, "插画");
        assert_eq!(illust.width, Some(4000));
        let original = illust.meta_pages[0].image_urls.original.as_deref().unwrap();
        assert_eq!(original, "https://i0.hdslb.com/bfs/new_dyn/a.jpg");
        assert!(illust.meta_pages[0]
            .image_urls
            .large
            .as_deref()
            .unwrap()
            .ends_with("@1280w_1280h_1c.jpg"));
        assert_eq!(
            illust
                .provider_data
                .as_ref()
                .unwrap()
                .comment_oid
                .as_deref(),
            Some("395446267")
        );
        assert_eq!(
            dynamic_web_url(&illust),
            "https://t.bilibili.com/1205134739172753431"
        );
    }

    #[test]
    fn rejects_text_only_and_video() {
        let mut text = sample_dynamic();
        text["modules"]["module_dynamic"]["major"] = json!({"type": "MAJOR_TYPE_NONE"});
        assert!(dynamic_to_illust(&text).is_none());

        let video = json!({
            "id_str": "1",
            "type": "DYNAMIC_TYPE_AV",
            "modules": {
                "module_dynamic": {"major": {"type": "MAJOR_TYPE_ARCHIVE", "archive": {"aid": "1"}}}
            }
        });
        assert!(dynamic_to_illust(&video).is_none());
    }

    #[test]
    fn forward_unwraps_to_original() {
        let original = sample_dynamic();
        let forward = json!({
            "id_str": "999",
            "type": "DYNAMIC_TYPE_FORWARD",
            "basic": {"comment_id_str": "111"},
            "modules": {"module_dynamic": {"desc": {"text": "转发动态"}}},
            "orig": original
        });
        let illust = dynamic_to_illust(&forward).expect("original has photos");
        assert_eq!(illust.id, bilibili_key(1205134739172753431));
        assert_eq!(illust.caption.as_deref(), Some("first line\nsecond line"));
    }

    #[test]
    fn maps_reply() {
        let reply = json!({
            "rpid": 123456789,
            "ctime": "1700000000",
            "content": {"message": "nice!"},
            "member": {"mid": "518833285", "uname": "someone", "avatar": "http://i1.hdslb.com/bfs/face/x.jpg"},
            "rcount": 2
        });
        let c = reply_to_comment(&reply).unwrap();
        assert_eq!(c.id, bilibili_key(123456789));
        assert_eq!(c.comment, "nice!");
        assert_eq!(c.user.as_ref().unwrap().id, bilibili_key(518833285));
        assert_eq!(c.has_replies, Some(true));
    }
}

//! Scrape the current GraphQL query ids from X's web bundles.
//!
//! In-app port of what used to be `scripts/refresh-twitter-query-ids.sh`:
//!
//! 1. Fetch `x.com/home` with the user's cookies.
//! 2. Find the entry `main.<hash>.js` bundle and the lazy chunks whose
//!    webpack name mentions a timeline (`HomeTimeline`/`HomeLatestTimeline`
//!    do not live in `main.js`).
//! 3. Extract every `queryId:"…",operationName:"…"` pair and keep the
//!    operations the app calls.

use super::TwitterError;
use std::collections::{HashMap, HashSet};

/// Where the web client's JS bundles are served from.
const BUNDLE_BASE: &str = "https://abs.twimg.com/responsive-web/client-web";
/// Prefix of the entry bundle URL embedded in the home HTML.
const MAIN_PREFIX: &str = "https://abs.twimg.com/responsive-web/client-web/main.";
/// The home page whose HTML references the bundles.
const HOME_URL: &str = "https://x.com/home";

/// Download the current web bundles and extract the query ids for the
/// operations the app uses. Operations missing from the bundles are simply
/// absent from the result (callers keep their existing value).
pub async fn fetch(
    http: &reqwest::Client,
    cookie_header: &str,
) -> Result<HashMap<String, String>, TwitterError> {
    let home = get_text(http, HOME_URL, cookie_header).await?;
    let main_url = find_main_bundle(&home).ok_or_else(|| {
        TwitterError::Internal(
            "could not find main.js in x.com/home (expired cookies or a login wall?)".into(),
        )
    })?;
    let mut scripts = vec![get_text(http, &main_url, cookie_header).await?];
    for url in timeline_chunk_urls(&home) {
        scripts.push(get_text(http, &url, cookie_header).await?);
    }

    let wanted: HashSet<&str> = super::graphql::DEFAULT_QUERIES
        .iter()
        .map(|(operation, _)| *operation)
        .collect();
    let mut pairs = HashMap::new();
    for script in &scripts {
        extract_pairs(script, &mut pairs);
    }
    pairs.retain(|operation, _| wanted.contains(operation.as_str()));
    if pairs.is_empty() {
        return Err(TwitterError::Internal(
            "no query ids found in the web bundles; X may have changed its bundle format".into(),
        ));
    }
    Ok(pairs)
}

async fn get_text(
    http: &reqwest::Client,
    url: &str,
    cookie_header: &str,
) -> Result<String, TwitterError> {
    let resp = http
        .get(url)
        .header("user-agent", super::USER_AGENT)
        .header("accept", "*/*")
        .header("cookie", cookie_header)
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(TwitterError::HttpStatus(
            resp.status().as_u16(),
            format!("GET {url}"),
        ));
    }
    Ok(resp.text().await?)
}

/// The `main.<hash>.js` entry-bundle URL referenced by the home HTML.
fn find_main_bundle(html: &str) -> Option<String> {
    let start = html.find(MAIN_PREFIX)?;
    let rest = &html[start..];
    let end = rest.find(".js")? + ".js".len();
    Some(rest[..end].to_string())
}

/// URLs of the lazy chunks whose webpack name mentions a timeline. Title
/// operations (`HomeTimeline`, `HomeLatestTimeline`) are not in `main.js`.
fn timeline_chunk_urls(html: &str) -> Vec<String> {
    let mut names: Vec<(String, String)> = Vec::new();
    let mut seen = HashSet::new();
    for (quote_at, name) in quoted_tokens(html) {
        if !name.contains("Timeline") || !name.chars().all(is_chunk_name_char) {
            continue;
        }
        // The webpack name map is `{id}:"{name}"` (minified, so the digit run
        // sits immediately before the colon).
        let before = &html[..quote_at];
        if !before.ends_with(':') {
            continue;
        }
        let digits = &before[..before.len() - 1];
        let id_start = digits
            .rfind(|c: char| !c.is_ascii_digit())
            .map(|i| i + 1)
            .unwrap_or(0);
        let id = &digits[id_start..];
        if id.is_empty() || !seen.insert(name.to_string()) {
            continue;
        }
        names.push((id.to_string(), name.to_string()));
    }

    names
        .into_iter()
        .filter_map(|(id, name)| {
            let hash = find_chunk_hash(html, &id)?;
            Some(format!("{BUNDLE_BASE}/{name}.{hash}a.js"))
        })
        .collect()
}

/// Every `"…"` token in `s` as `(byte index of the opening quote, value)`.
fn quoted_tokens(s: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'"' {
            i += 1;
            continue;
        }
        let start = i;
        i += 1;
        while i < bytes.len() && bytes[i] != b'"' {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        out.push((start, &s[start + 1..i]));
        i += 1;
    }
    out
}

fn is_chunk_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '~' | '-')
}

/// The 16-hex content hash webpack pairs with a chunk id (`{id}:"{hash}"`).
/// The id can appear in several maps, so scan until a value matches the
/// expected shape.
fn find_chunk_hash(html: &str, id: &str) -> Option<String> {
    let needle = format!("{id}:\"");
    let mut from = 0;
    while let Some(rel) = html[from..].find(&needle) {
        let value_at = from + rel + needle.len();
        let rest = &html[value_at..];
        if let Some(end) = rest.find('"') {
            let hash = &rest[..end];
            if hash.len() == 16 && hash.chars().all(|c| c.is_ascii_hexdigit()) {
                return Some(hash.to_string());
            }
        }
        from = value_at;
    }
    None
}

/// Extract every `queryId:"{id}",operationName:"{name}"` pair into `out`.
fn extract_pairs(js: &str, out: &mut HashMap<String, String>) {
    const ID_KEY: &str = "queryId:\"";
    const NAME_KEY: &str = ",operationName:\"";
    let mut rest = js;
    while let Some(pos) = rest.find(ID_KEY) {
        rest = &rest[pos + ID_KEY.len()..];
        let Some(end) = rest.find('"') else { break };
        let id = &rest[..end];
        rest = &rest[end + 1..];
        let Some(after) = rest.strip_prefix(NAME_KEY) else {
            continue;
        };
        let Some(name_end) = after.find('"') else {
            break;
        };
        let name = &after[..name_end];
        if is_query_id(id) && is_operation_name(name) {
            out.entry(name.to_string())
                .or_insert_with(|| id.to_string());
        }
        rest = &after[name_end + 1..];
    }
}

fn is_query_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn is_operation_name(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A trimmed-down home page with the webpack maps the parser reads.
    fn home_html() -> String {
        let mut html = String::from(
            r#"<html><script src="https://abs.twimg.com/responsive-web/client-web/main.abc123.js"></script>"#,
        );
        // Name map, then hash map (the order the real page uses; webpack
        // minifies with unquoted numeric keys).
        html.push_str(r#"<script>{123:"shared~bundle.Compose~bundle.HomeTimeline",124:"bundle.HomeLatestTimeline"}</script>"#);
        html.push_str(r#"<script>{123:"0123456789abcdef",124:"fedcba9876543210"}</script>"#);
        html
    }

    #[test]
    fn finds_the_main_bundle() {
        assert_eq!(
            find_main_bundle(&home_html()).as_deref(),
            Some("https://abs.twimg.com/responsive-web/client-web/main.abc123.js")
        );
        assert!(find_main_bundle("<html>no bundles</html>").is_none());
    }

    #[test]
    fn finds_timeline_chunk_urls() {
        let urls = timeline_chunk_urls(&home_html());
        assert_eq!(
            urls,
            vec![
                format!(
                    "{BUNDLE_BASE}/shared~bundle.Compose~bundle.HomeTimeline.0123456789abcdefa.js"
                ),
                format!("{BUNDLE_BASE}/bundle.HomeLatestTimeline.fedcba9876543210a.js"),
            ]
        );
    }

    #[test]
    fn extracts_query_id_pairs() {
        let js = r#"a,{queryId:"abc-DEF_123",operationName:"HomeTimeline"},b,{queryId:"XYZ",operationName:"Viewer"},"#;
        let mut pairs = HashMap::new();
        extract_pairs(js, &mut pairs);
        assert_eq!(
            pairs.get("HomeTimeline").map(String::as_str),
            Some("abc-DEF_123")
        );
        assert_eq!(pairs.get("Viewer").map(String::as_str), Some("XYZ"));
    }

    #[test]
    fn malformed_pairs_are_ignored() {
        let js = r#"queryId:"has spaces",operationName:"Bad" queryId:"ok",operationName:"not valid!" queryId:"",operationName:"Empty""#;
        let mut pairs = HashMap::new();
        extract_pairs(js, &mut pairs);
        assert!(pairs.is_empty(), "got {pairs:?}");
    }

    #[test]
    fn keeps_the_first_id_for_duplicate_operations() {
        let js = r#"queryId:"FIRST",operationName:"Likes" queryId:"SECOND",operationName:"Likes""#;
        let mut pairs = HashMap::new();
        extract_pairs(js, &mut pairs);
        assert_eq!(pairs.get("Likes").map(String::as_str), Some("FIRST"));
    }
}

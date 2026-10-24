//! Danbooru-style text queries: `tag -excluded ~either ~or user:name
//! views:>1000 is:r18 order:bookmarks`.
//!
//! A query is a whitespace-separated list of tokens (double quotes keep a
//! value with spaces together). Plain tokens are exact tag matches (`*` is a
//! wildcard, `_` stands for a space); `name:value` tokens are metatags. A
//! leading `-` negates a token and a leading `~` puts it in the "any of"
//! group. `order:` tokens control sorting instead of filtering.
//!
//! The query is parsed with [`Query::parse`] into [`Token`]s (so the UI can
//! show them as chips, including the ones that failed to parse) and evaluated
//! per illustration with [`Query::matches`].

use crate::app::sort::{SortDirection, SortKey, SortState};
use crate::media::Provider;
use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use pixivj_rs::model::Illustration;
use std::collections::HashSet;

/// Numeric fields a metatag can compare.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumField {
    Views,
    Bookmarks,
    /// Bookmarks ÷ views, as a fraction (`bpv:>5%` == `bpv:>0.05`).
    BookmarksPerView,
    Width,
    Height,
    Pixels,
    Pages,
    Sanity,
    /// Width ÷ height.
    Ratio,
    /// The native (provider-local) id.
    Id,
    /// Seconds since the work was posted (written as `7d`, `2w`, …).
    Age,
    /// Posting time as unix seconds (written as dates).
    Date,
}

/// An inclusive/exclusive numeric interval; `None` bounds are open.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Range {
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub min_exclusive: bool,
    pub max_exclusive: bool,
}

impl Range {
    fn contains(&self, v: f64) -> bool {
        let lo = match self.min {
            Some(m) if self.min_exclusive => v > m,
            Some(m) => v >= m,
            None => true,
        };
        let hi = match self.max {
            Some(m) if self.max_exclusive => v < m,
            Some(m) => v <= m,
            None => true,
        };
        lo && hi
    }
}

/// Boolean attributes for `is:`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flag {
    R18,
    Ai,
    Bookmarked,
    Landscape,
    Portrait,
    Visible,
    Restricted,
    /// Not downloaded locally.
    Missing,
    Downloaded,
    Multi,
}

/// One thing an illustration is tested against.
#[derive(Debug, Clone, PartialEq)]
pub enum Cond {
    /// Exact tag (name or translation), `*` wildcards; already normalized.
    Tag(String),
    Author(String),
    Title(String),
    Caption(String),
    Tool(String),
    Source(Provider),
    Flag(Flag),
    Num(NumField, Range),
}

/// What a token does.
#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    /// Must match.
    Include(Cond),
    /// Must not match.
    Exclude(Cond),
    /// At least one of all `~` tokens must match.
    AnyOf(Cond),
    /// Sort rule.
    Order(SortKey, SortDirection),
    /// A metatag with a value that could not be understood (ignored).
    Invalid(String),
}

/// One parsed token: its source text and meaning.
#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub raw: String,
    pub kind: Kind,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Query {
    pub tokens: Vec<Token>,
}

/// Context an evaluation needs besides the illustration.
pub struct Context<'a> {
    pub now: DateTime<Utc>,
    pub downloaded: &'a HashSet<u64>,
}

// ── Tokenizing ─────────────────────────────────────────────────────────

/// Split on whitespace, keeping `"quoted values"` together (quotes are kept
/// in the token so [`Query::without_token`] round-trips).
pub fn split_tokens(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for c in text.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                cur.push(c);
            }
            c if c.is_whitespace() && !quoted => {
                if !cur.is_empty() {
                    tokens.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        tokens.push(cur);
    }
    tokens
}

/// Lowercase, `_` → space, quotes dropped — the form tags and patterns are
/// compared in.
fn normalize(s: &str) -> String {
    s.replace('"', "").replace('_', " ").trim().to_lowercase()
}

/// `*` wildcard match over normalized strings.
fn glob_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    let (mut pi, mut ti) = (0usize, 0usize);
    let (mut star, mut mark) = (None::<usize>, 0usize);
    while ti < t.len() {
        if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if pi < p.len() && p[pi] == t[ti] {
            pi += 1;
            ti += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

// ── Value parsing ──────────────────────────────────────────────────────

/// A number with optional `k`/`m` (thousand/million) or `%` suffix.
fn parse_number(s: &str) -> Option<f64> {
    let s = s.trim().to_lowercase();
    let (body, scale) = if let Some(b) = s.strip_suffix('%') {
        (b, 0.01)
    } else if let Some(b) = s.strip_suffix('k') {
        (b, 1_000.0)
    } else if let Some(b) = s.strip_suffix('m') {
        (b, 1_000_000.0)
    } else {
        (s.as_str(), 1.0)
    };
    body.parse::<f64>().ok().map(|v| v * scale)
}

/// A duration such as `90m`, `12h`, `7d`, `2w`, `6mo`, `1y` in seconds
/// (bare numbers are days).
fn parse_age(s: &str) -> Option<f64> {
    let s = s.trim().to_lowercase();
    let split = s.find(|c: char| c.is_alphabetic()).unwrap_or(s.len());
    let (num, unit) = s.split_at(split);
    let n: f64 = num.parse().ok()?;
    let secs = match unit {
        "" | "d" => 86_400.0,
        "h" => 3_600.0,
        "m" | "min" => 60.0,
        "w" => 7.0 * 86_400.0,
        "mo" => 30.0 * 86_400.0,
        "y" => 365.0 * 86_400.0,
        _ => return None,
    };
    Some(n * secs)
}

/// A date (`2024`, `2024-05`, `2024-05-17`) as `[start, end)` unix seconds.
fn parse_date_span(s: &str) -> Option<(f64, f64)> {
    let parts: Vec<&str> = s.trim().split(['-', '/']).collect();
    let num = |i: usize| parts.get(i).and_then(|p| p.parse::<u32>().ok());
    let year = num(0)? as i32;
    let ts = |d: NaiveDate| {
        Utc.from_utc_datetime(&d.and_hms_opt(0, 0, 0).unwrap())
            .timestamp() as f64
    };
    match parts.len() {
        1 => Some((
            ts(NaiveDate::from_ymd_opt(year, 1, 1)?),
            ts(NaiveDate::from_ymd_opt(year + 1, 1, 1)?),
        )),
        2 => {
            let m = num(1)?;
            let start = NaiveDate::from_ymd_opt(year, m, 1)?;
            let end = if m == 12 {
                NaiveDate::from_ymd_opt(year + 1, 1, 1)?
            } else {
                NaiveDate::from_ymd_opt(year, m + 1, 1)?
            };
            Some((ts(start), ts(end)))
        }
        3 => {
            let start = NaiveDate::from_ymd_opt(year, num(1)?, num(2)?)?;
            Some((ts(start), ts(start) + 86_400.0))
        }
        _ => None,
    }
}

/// Parse a comparison value for `field`: `N`, `>N`, `>=N`, `<N`, `<=N`,
/// `=N`, `A..B`, `A..`, `..B`.
fn parse_range(field: NumField, value: &str) -> Option<Range> {
    // Single scalar → (low, high) bounds of what it denotes. Dates denote a
    // span (`2024` is the whole year); everything else is a point.
    let scalar = |s: &str| -> Option<(f64, f64)> {
        match field {
            NumField::Date => parse_date_span(s),
            NumField::Age => parse_age(s).map(|v| (v, v)),
            _ => parse_number(s).map(|v| (v, v)),
        }
    };
    let open = Range {
        min: None,
        max: None,
        min_exclusive: false,
        max_exclusive: false,
    };
    // Dates are half-open spans: [start, end).
    let is_date = field == NumField::Date;
    if let Some((a, b)) = value.split_once("..") {
        let min = if a.is_empty() {
            None
        } else {
            Some(scalar(a)?.0)
        };
        let (max, max_exclusive) = if b.is_empty() {
            (None, false)
        } else {
            let (lo, hi) = scalar(b)?;
            if is_date {
                (Some(hi), true)
            } else {
                (Some(lo), false)
            }
        };
        return Some(Range {
            min,
            max,
            max_exclusive,
            ..open
        });
    }
    let (op, rest) = ["<=", ">=", "<", ">", "="]
        .iter()
        .find_map(|op| value.strip_prefix(op).map(|r| (*op, r)))
        .unwrap_or(("=", value));
    let (lo, hi) = scalar(rest)?;
    Some(match op {
        ">" => Range {
            min: Some(if is_date { hi } else { lo }),
            min_exclusive: !is_date,
            ..open
        },
        ">=" => Range {
            min: Some(lo),
            ..open
        },
        "<" => Range {
            max: Some(lo),
            max_exclusive: true,
            ..open
        },
        "<=" => Range {
            max: Some(if is_date { hi } else { lo }),
            max_exclusive: is_date,
            ..open
        },
        _ if is_date => Range {
            min: Some(lo),
            max: Some(hi),
            max_exclusive: true,
            ..open
        },
        _ => Range {
            min: Some(lo),
            max: Some(lo),
            ..open
        },
    })
}

fn num_field(name: &str) -> Option<NumField> {
    Some(match name {
        "views" | "view" => NumField::Views,
        "bookmarks" | "bookmark" | "favcount" | "favs" | "score" => NumField::Bookmarks,
        "bpv" => NumField::BookmarksPerView,
        "width" => NumField::Width,
        "height" => NumField::Height,
        "pixels" | "mpixels" => NumField::Pixels,
        "pages" | "page_count" => NumField::Pages,
        "sanity" => NumField::Sanity,
        "ratio" => NumField::Ratio,
        "id" => NumField::Id,
        "age" => NumField::Age,
        "date" => NumField::Date,
        _ => return None,
    })
}

fn flag(name: &str) -> Option<Flag> {
    Some(match name {
        "r18" | "r-18" | "nsfw" => Flag::R18,
        "ai" => Flag::Ai,
        "bookmarked" | "bookmark" | "liked" => Flag::Bookmarked,
        "landscape" | "wide" => Flag::Landscape,
        "portrait" | "tall" => Flag::Portrait,
        "visible" => Flag::Visible,
        "restricted" => Flag::Restricted,
        "missing" => Flag::Missing,
        "downloaded" => Flag::Downloaded,
        "multi" | "multipage" => Flag::Multi,
        _ => return None,
    })
}

fn source(name: &str) -> Option<Provider> {
    Some(match name {
        "pixiv" => Provider::Pixiv,
        "twitter" | "x" => Provider::Twitter,
        "bilibili" | "bili" => Provider::Bilibili,
        _ => return None,
    })
}

fn order(value: &str) -> Option<(SortKey, SortDirection)> {
    let (name, dir) = match value.strip_suffix("_asc") {
        Some(n) => (n, SortDirection::Up),
        None => (
            value.strip_suffix("_desc").unwrap_or(value),
            SortDirection::Down,
        ),
    };
    let key = match name {
        "views" | "view" => SortKey::Views,
        "bookmarks" | "favcount" | "favs" | "score" => SortKey::Bookmarks,
        "bpv" => SortKey::BookmarksPerView,
        "date" => SortKey::Date,
        "id" => SortKey::Id,
        "sanity" => SortKey::Sanity,
        "pixels" | "size" => SortKey::Pixel,
        "pages" => SortKey::Pages,
        _ => return None,
    };
    Some((key, dir))
}

/// Metatag names offered by the editor's suggestions.
pub const METATAGS: &[&str] = &[
    "user",
    "title",
    "caption",
    "tool",
    "source",
    "is",
    "order",
    "views",
    "bookmarks",
    "bpv",
    "width",
    "height",
    "pixels",
    "pages",
    "sanity",
    "ratio",
    "age",
    "date",
    "id",
];

const FLAG_VALUES: &[&str] = &[
    "r18",
    "ai",
    "bookmarked",
    "landscape",
    "portrait",
    "visible",
    "restricted",
    "missing",
    "downloaded",
    "multi",
];
const SOURCE_VALUES: &[&str] = &["pixiv", "twitter", "bilibili"];
const ORDER_VALUES: &[&str] = &[
    "views",
    "bookmarks",
    "bpv",
    "date",
    "id",
    "pixels",
    "pages",
    "sanity",
    "views_asc",
    "bookmarks_asc",
    "date_asc",
    "id_asc",
];

fn parse_cond(name: &str, value: &str) -> Result<Option<Cond>, String> {
    let nv = normalize(value);
    let empty = || format!("{name}: needs a value");
    Ok(Some(match name {
        "user" | "author" | "artist" => {
            if nv.is_empty() {
                return Err(empty());
            }
            Cond::Author(nv)
        }
        "title" => {
            if nv.is_empty() {
                return Err(empty());
            }
            Cond::Title(nv)
        }
        "caption" | "desc" | "description" => {
            if nv.is_empty() {
                return Err(empty());
            }
            Cond::Caption(nv)
        }
        "tool" => {
            if nv.is_empty() {
                return Err(empty());
            }
            Cond::Tool(nv)
        }
        "source" | "site" => Cond::Source(
            source(&nv)
                .ok_or_else(|| format!("source: unknown '{value}' (pixiv, twitter, bilibili)"))?,
        ),
        "is" | "has" => Cond::Flag(
            flag(&nv)
                .ok_or_else(|| format!("is: unknown '{value}' ({})", FLAG_VALUES.join(", ")))?,
        ),
        other => match num_field(other) {
            Some(field) => {
                // `pixels` is entered in megapixels when it has no unit? Keep
                // raw pixels, but `mpixels:` means megapixels.
                let mut range = parse_range(field, &nv).ok_or_else(|| {
                    format!("{other}: can't read '{value}' (try >100, 10..50, >=2k)")
                })?;
                if other == "mpixels" {
                    for b in [&mut range.min, &mut range.max].into_iter().flatten() {
                        *b *= 1_000_000.0;
                    }
                }
                Cond::Num(field, range)
            }
            None => return Ok(None),
        },
    }))
}

impl Query {
    /// Parse `text` into tokens. Never fails: bad metatag values become
    /// [`Kind::Invalid`] tokens (ignored when matching).
    pub fn parse(text: &str) -> Self {
        let tokens = split_tokens(text)
            .into_iter()
            .filter_map(|raw| {
                let kind = Self::parse_token(&raw)?;
                Some(Token { raw, kind })
            })
            .collect();
        Self { tokens }
    }

    fn parse_token(raw: &str) -> Option<Kind> {
        let (negate, any, body) = if let Some(b) = raw.strip_prefix('-') {
            (true, false, b)
        } else if let Some(b) = raw.strip_prefix('~') {
            (false, true, b)
        } else {
            (false, false, raw)
        };
        if body.is_empty() || body == "\"" {
            return None;
        }
        let wrap = |c: Cond| {
            if negate {
                Kind::Exclude(c)
            } else if any {
                Kind::AnyOf(c)
            } else {
                Kind::Include(c)
            }
        };
        if let Some((name, value)) = body.split_once(':') {
            let name = name.to_lowercase();
            if name == "order" {
                return Some(match order(&value.trim().to_lowercase()) {
                    Some((key, dir)) if !negate && !any => Kind::Order(key, dir),
                    Some(_) => Kind::Invalid("order: can't be negated".into()),
                    None => Kind::Invalid(format!(
                        "order: unknown '{value}' ({})",
                        ORDER_VALUES.join(", ")
                    )),
                });
            }
            match parse_cond(&name, value) {
                Ok(Some(cond)) => return Some(wrap(cond)),
                Ok(None) => {} // not a metatag: treat the whole token as a tag
                Err(reason) => return Some(Kind::Invalid(reason)),
            }
        }
        Some(wrap(Cond::Tag(normalize(body))))
    }

    /// Whether the query constrains or sorts anything.
    pub fn is_empty(&self) -> bool {
        !self
            .tokens
            .iter()
            .any(|t| !matches!(t.kind, Kind::Invalid(_)))
    }

    /// Whether any token mentions the `is:bookmarked` flag (either polarity).
    pub fn mentions_bookmarked(&self) -> bool {
        self.tokens.iter().any(|t| {
            matches!(
                &t.kind,
                Kind::Include(Cond::Flag(Flag::Bookmarked))
                    | Kind::Exclude(Cond::Flag(Flag::Bookmarked))
                    | Kind::AnyOf(Cond::Flag(Flag::Bookmarked))
            )
        })
    }

    /// The sort rules, in the order written.
    pub fn sort_state(&self) -> SortState {
        SortState {
            multi_sort: true,
            active: self
                .tokens
                .iter()
                .filter_map(|t| match t.kind {
                    Kind::Order(k, d) => Some((k, d)),
                    _ => None,
                })
                .collect(),
            ..Default::default()
        }
    }

    /// Plain positive tags as a server-side search string (what the Search
    /// tab sends to Pixiv): the `Include` tags without wildcards, `_` kept
    /// as written by the user.
    pub fn server_terms(&self) -> String {
        self.tokens
            .iter()
            .filter_map(|t| match &t.kind {
                Kind::Include(Cond::Tag(tag)) if !tag.contains('*') => Some(tag.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// The query text with token `index` removed.
    pub fn without_token(&self, index: usize) -> String {
        self.tokens
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != index)
            .map(|(_, t)| t.raw.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Whether `i` satisfies every filter token. `order:` and invalid tokens
    /// don't filter.
    pub fn matches(&self, i: &Illustration, ctx: &Context) -> bool {
        let mut any_seen = false;
        let mut any_hit = false;
        for token in &self.tokens {
            match &token.kind {
                Kind::Include(c) => {
                    if !cond_matches(c, i, ctx) {
                        return false;
                    }
                }
                Kind::Exclude(c) => {
                    if cond_matches(c, i, ctx) {
                        return false;
                    }
                }
                Kind::AnyOf(c) => {
                    any_seen = true;
                    any_hit |= cond_matches(c, i, ctx);
                }
                Kind::Order(..) | Kind::Invalid(_) => {}
            }
        }
        !any_seen || any_hit
    }
}

fn cond_matches(cond: &Cond, i: &Illustration, ctx: &Context) -> bool {
    let contains =
        |hay: Option<&str>, needle: &str| hay.is_some_and(|h| normalize(h).contains(needle));
    match cond {
        Cond::Tag(pattern) => i.tags.iter().any(|t| {
            glob_match(pattern, &normalize(&t.name))
                || t.translated_name
                    .as_deref()
                    .is_some_and(|tn| glob_match(pattern, &normalize(tn)))
        }),
        Cond::Author(n) => contains(i.user.as_ref().map(|u| u.name.as_str()), n),
        Cond::Title(n) => contains(i.title.as_deref(), n),
        Cond::Caption(n) => contains(i.caption.as_deref(), n),
        Cond::Tool(n) => i.tools.iter().any(|t| normalize(t).contains(n)),
        Cond::Source(p) => crate::media::provider_of(i.id) == *p,
        Cond::Flag(f) => match f {
            Flag::R18 => i.is_r18(),
            Flag::Ai => i.is_ai(),
            Flag::Bookmarked => i.bookmarked(),
            Flag::Landscape => i.is_landscape(),
            Flag::Portrait => i.width.unwrap_or(0) < i.height.unwrap_or(0),
            Flag::Visible => i.is_visible(),
            Flag::Restricted => i.is_restricted(),
            Flag::Downloaded => ctx.downloaded.contains(&i.id),
            Flag::Missing => !ctx.downloaded.contains(&i.id),
            Flag::Multi => i.page_count() > 1,
        },
        Cond::Num(field, range) => {
            let value = match field {
                NumField::Views => i.total_view.unwrap_or(0) as f64,
                NumField::Bookmarks => i.total_bookmarks.unwrap_or(0) as f64,
                NumField::BookmarksPerView => {
                    let views = i.total_view.unwrap_or(0) as f64;
                    if views > 0.0 {
                        i.total_bookmarks.unwrap_or(0) as f64 / views
                    } else {
                        0.0
                    }
                }
                NumField::Width => i.width.unwrap_or(0) as f64,
                NumField::Height => i.height.unwrap_or(0) as f64,
                NumField::Pixels => i.width.unwrap_or(0) as f64 * i.height.unwrap_or(0) as f64,
                NumField::Pages => i.page_count() as f64,
                NumField::Sanity => i.sanity_level.unwrap_or(0) as f64,
                NumField::Ratio => {
                    let h = i.height.unwrap_or(0) as f64;
                    if h > 0.0 {
                        i.width.unwrap_or(0) as f64 / h
                    } else {
                        0.0
                    }
                }
                NumField::Id => crate::media::native_id(i.id) as f64,
                NumField::Age => match i.create_date {
                    Some(d) => (ctx.now - d).num_seconds() as f64,
                    None => return false,
                },
                NumField::Date => match i.create_date {
                    Some(d) => d.timestamp() as f64,
                    None => return false,
                },
            };
            range.contains(value)
        }
    }
}

// ── Suggestions & editing ──────────────────────────────────────────────

/// Completions for the token being typed at the end of `text` (nothing when
/// the text ends in whitespace). Each suggestion replaces that last token
/// entirely, keeping its `-`/`~` prefix; metatag names end in `:`.
pub fn suggest(text: &str, gallery_tags: &[String]) -> Vec<String> {
    if text.is_empty() || text.ends_with(char::is_whitespace) {
        return Vec::new();
    }
    let Some(last) = split_tokens(text).pop() else {
        return Vec::new();
    };
    let (prefix, body) = match last.chars().next() {
        Some(c @ ('-' | '~')) => (c.to_string(), &last[1..]),
        _ => (String::new(), last.as_str()),
    };
    if body.is_empty() {
        return Vec::new();
    }
    let lower = body.to_lowercase();
    let with_prefix = |s: String| format!("{prefix}{s}");
    // Value completion for `is:` / `source:` / `order:`.
    if let Some((name, value)) = lower.split_once(':') {
        let values: &[&str] = match name {
            "is" | "has" => FLAG_VALUES,
            "source" | "site" => SOURCE_VALUES,
            "order" => ORDER_VALUES,
            _ => return Vec::new(),
        };
        return values
            .iter()
            .filter(|v| v.starts_with(value) && **v != value)
            .map(|v| with_prefix(format!("{name}:{v}")))
            .collect();
    }
    let wanted = normalize(body);
    let mut out: Vec<String> = METATAGS
        .iter()
        .filter(|m| m.starts_with(&lower))
        .map(|m| with_prefix(format!("{m}:")))
        .collect();
    let mut seen = HashSet::new();
    let mut starts: Vec<String> = Vec::new();
    let mut contains: Vec<String> = Vec::new();
    for tag in gallery_tags {
        let n = normalize(tag);
        if n == wanted || !seen.insert(n.clone()) {
            continue;
        }
        let token = with_prefix(tag.trim().replace(' ', "_"));
        if n.starts_with(&wanted) {
            starts.push(token);
        } else if n.contains(&wanted) {
            contains.push(token);
        }
    }
    out.extend(starts);
    out.extend(contains);
    out.truncate(8);
    out
}

/// Replace the token being typed at the end of `text` with `suggestion`
/// (a trailing space is added unless it is a metatag name ending in `:`).
pub fn apply_suggestion(text: &str, suggestion: &str) -> String {
    let head = match text.rfind(char::is_whitespace) {
        Some(pos) => &text[..=pos],
        None => "",
    };
    if suggestion.ends_with(':') {
        format!("{head}{suggestion}")
    } else {
        format!("{head}{suggestion} ")
    }
}

// ── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use pixivj_rs::model::{Author, Tag};

    fn illust(id: u64) -> Illustration {
        crate::test_util::mock_illust(id)
    }

    fn tagged(id: u64, tags: &[&str]) -> Illustration {
        let mut i = illust(id);
        i.tags = tags
            .iter()
            .map(|t| Tag {
                name: t.to_string(),
                translated_name: None,
            })
            .collect();
        i
    }

    fn ctx(downloaded: &HashSet<u64>) -> Context<'_> {
        Context {
            now: Utc.with_ymd_and_hms(2025, 6, 15, 12, 0, 0).unwrap(),
            downloaded,
        }
    }

    fn run(q: &str, i: &Illustration) -> bool {
        let none = HashSet::new();
        Query::parse(q).matches(i, &ctx(&none))
    }

    #[test]
    fn tokenizer_respects_quotes() {
        assert_eq!(
            split_tokens(r#"a  title:"two words" -b"#),
            vec!["a", r#"title:"two words""#, "-b"]
        );
    }

    #[test]
    fn glob_basics() {
        assert!(glob_match("blue*", "blue sky"));
        assert!(glob_match("*sky", "blue sky"));
        assert!(glob_match("*u*k*", "blue sky"));
        assert!(!glob_match("blue", "blue sky"));
        assert!(glob_match("*", ""));
    }

    #[test]
    fn tags_include_exclude_any() {
        let i = tagged(1, &["Blue Sky", "girl"]);
        assert!(run("girl", &i));
        assert!(run("blue_sky girl", &i));
        assert!(run("BLUE_SKY", &i));
        assert!(!run("blue", &i), "plain tags match exactly");
        assert!(run("blue*", &i));
        assert!(run("-boy", &i));
        assert!(!run("-girl", &i));
        assert!(run("~boy ~girl", &i));
        assert!(!run("~boy ~dog", &i));
        assert!(run("girl ~boy ~girl", &i));
        assert!(run("", &i));
    }

    #[test]
    fn translated_tag_names_match() {
        let mut i = illust(1);
        i.tags = vec![Tag {
            name: "風景".into(),
            translated_name: Some("scenery".into()),
        }];
        assert!(run("scenery", &i));
        assert!(run("風景", &i));
    }

    #[test]
    fn numeric_ranges() {
        let mut i = illust(1);
        i.total_view = Some(1500);
        i.total_bookmarks = Some(150);
        i.width = Some(2000);
        i.height = Some(1000);
        i.page_count = Some(3);
        assert!(run("views:>1000", &i));
        assert!(run("views:>=1.5k", &i));
        assert!(!run("views:>1500", &i));
        assert!(run("views:1000..2000", &i));
        assert!(run("views:..1500", &i));
        assert!(run("views:1500", &i));
        assert!(run("bookmarks:<200 -bookmarks:<100", &i));
        assert!(run("bpv:>=10%", &i));
        assert!(run("bpv:0.1", &i));
        assert!(run("ratio:2", &i));
        assert!(run("width:>=2k height:<=1k pages:>1", &i));
        assert!(run("pixels:2m", &i));
        assert!(run("-views:>5k", &i));
    }

    #[test]
    fn flags_and_sources() {
        let mut i = illust(1);
        i.x_restrict = Some(1);
        i.is_bookmarked = Some(true);
        i.width = Some(100);
        i.height = Some(200);
        assert!(run("is:r18 is:bookmarked is:portrait", &i));
        assert!(run("-is:ai -is:landscape", &i));
        assert!(run("source:pixiv", &i));
        assert!(!run("source:twitter", &i));
        let mut tw = illust(1);
        tw.id = crate::media::twitter_key(9);
        assert!(run("source:x", &tw));
        let mut downloaded = HashSet::new();
        let q = Query::parse("is:missing");
        assert!(q.matches(&i, &ctx(&downloaded)));
        downloaded.insert(1);
        assert!(!q.matches(&i, &ctx(&downloaded)));
        assert!(Query::parse("is:downloaded").matches(&i, &ctx(&downloaded)));
    }

    #[test]
    fn text_metatags() {
        let mut i = illust(1);
        i.title = Some("Summer Beach".into());
        i.user = Some(Author {
            id: 5,
            profile_image_urls: None,
            name: "Some Artist".into(),
            account: None,
            comment: None,
            is_followed: None,
        });
        assert!(run("title:beach", &i));
        assert!(run(r#"title:"summer beach""#, &i));
        assert!(run("title:summer_beach", &i));
        assert!(run("user:artist", &i));
        assert!(!run("user:other", &i));
        assert!(run("-user:other", &i));
    }

    #[test]
    fn dates_and_age() {
        let mut i = illust(1);
        i.create_date = Some(Utc.with_ymd_and_hms(2025, 6, 10, 8, 0, 0).unwrap());
        assert!(run("date:2025", &i));
        assert!(run("date:2025-06", &i));
        assert!(run("date:2025-06-10", &i));
        assert!(!run("date:2025-06-11", &i));
        assert!(run("date:>2025-06-09", &i));
        assert!(!run("date:>2025-06-10", &i));
        assert!(run("date:>=2025-06-10", &i));
        assert!(run("date:<2025-07", &i));
        assert!(run("date:2025-06-01..2025-06-30", &i));
        // "now" is 2025-06-15 12:00 → the work is 5 days 4 hours old.
        assert!(run("age:<7d", &i));
        assert!(!run("age:<5d", &i));
        assert!(!run("age:>1w", &i));
        assert!(run("age:>3d", &i));
        let mut undated = illust(2);
        undated.create_date = None;
        assert!(!run("age:<7d", &undated));
    }

    #[test]
    fn id_uses_native_id() {
        let mut tw = illust(1);
        tw.id = crate::media::twitter_key(12345);
        assert!(run("id:12345", &tw));
        assert!(run("id:>12000", &tw));
    }

    #[test]
    fn invalid_values_are_flagged_and_ignored() {
        let q = Query::parse("views:abc is:nonsense order:foo source:mars");
        assert_eq!(q.tokens.len(), 4);
        assert!(q.tokens.iter().all(|t| matches!(t.kind, Kind::Invalid(_))));
        assert!(q.is_empty());
        assert!(q.matches(&illust(1), &ctx(&HashSet::new())));
    }

    #[test]
    fn unknown_prefix_is_a_plain_tag() {
        let q = Query::parse("re:zero");
        assert_eq!(q.tokens[0].kind, Kind::Include(Cond::Tag("re:zero".into())));
    }

    #[test]
    fn order_tokens_build_sort_state() {
        let q = Query::parse("girl order:bookmarks order:id_asc");
        let sort = q.sort_state();
        assert_eq!(
            sort.active,
            vec![
                (SortKey::Bookmarks, SortDirection::Down),
                (SortKey::Id, SortDirection::Up)
            ]
        );
        assert!(
            Query::parse("-order:views").tokens[0].kind
                != Kind::Order(SortKey::Views, SortDirection::Down)
        );
    }

    #[test]
    fn server_terms_are_plain_positive_tags() {
        let q = Query::parse("blue_sky -boy ~a girl* views:>5 girl");
        assert_eq!(q.server_terms(), "blue sky girl");
    }

    #[test]
    fn without_token_roundtrips() {
        let q = Query::parse(r#"a -b title:"x y" c"#);
        assert_eq!(q.without_token(1), r#"a title:"x y" c"#);
    }

    #[test]
    fn bookmarked_mention_detection() {
        assert!(Query::parse("-is:bookmarked").mentions_bookmarked());
        assert!(!Query::parse("is:r18").mentions_bookmarked());
    }

    #[test]
    fn suggestions_complete_last_token() {
        let tags = vec![
            "blue sky".to_string(),
            "Blue Archive".to_string(),
            "dog".into(),
        ];
        let s = suggest("girl blu", &tags);
        assert_eq!(s, vec!["blue_sky", "Blue_Archive"]);
        assert_eq!(suggest("-blu", &tags), vec!["-blue_sky", "-Blue_Archive"]);
        assert!(suggest("blue ", &tags).is_empty());
        assert!(suggest("vi", &tags).contains(&"views:".to_string()));
        assert_eq!(suggest("is:r1", &tags), vec!["is:r18"]);
        assert_eq!(
            suggest("order:vi", &tags),
            vec!["order:views", "order:views_asc"]
        );
        assert!(suggest("blue_sky", &tags).iter().all(|s| s != "blue_sky"));
    }

    #[test]
    fn apply_suggestion_replaces_last_token() {
        assert_eq!(apply_suggestion("girl blu", "blue_sky"), "girl blue_sky ");
        assert_eq!(apply_suggestion("vi", "views:"), "views:");
        assert_eq!(apply_suggestion("a  is:r", "is:r18"), "a  is:r18 ");
    }

    #[test]
    fn query_mode_filters_and_sorts_through_the_reducer() {
        use crate::app::filter::FilterMode;
        use crate::app::message::Message;
        use crate::app::state::{AppState, Tab};

        let mut a = tagged(1, &["cat"]);
        a.total_view = Some(10);
        let mut b = tagged(2, &["cat"]);
        b.total_view = Some(500);
        let c = tagged(3, &["dog"]);
        let mut state = AppState::default();
        state.active_tab = Tab::Recommend;
        state.tabs.recommend.illustrations = vec![a, b, c];
        crate::app::update::update(&mut state, Message::FilterModeChanged(FilterMode::Query));
        crate::app::update::update(
            &mut state,
            Message::SearchTextChanged("cat order:views".into()),
        );
        let ids: Vec<u64> = state.tabs.recommend.filtered.iter().map(|i| i.id).collect();
        assert_eq!(ids, vec![2, 1]);

        // The classic search text is untouched and ignored in Query mode.
        assert!(state.tabs.recommend.filters.search_text.is_empty());

        // Removing the `cat` chip leaves only the order token: everything shows.
        crate::app::update::update(&mut state, Message::QueryTokenRemoved(0));
        assert_eq!(state.tabs.recommend.filters.query_text, "order:views");
        assert_eq!(state.tabs.recommend.filtered.len(), 3);

        // Clearing keeps the mode.
        crate::app::update::update(&mut state, Message::ResetFilters);
        assert_eq!(state.tabs.recommend.filters.mode, FilterMode::Query);
        assert!(state.tabs.recommend.filters.query_text.is_empty());

        // Back to Classic: the classic search box works again.
        crate::app::update::update(&mut state, Message::FilterModeChanged(FilterMode::Classic));
        crate::app::update::update(&mut state, Message::SearchTextChanged("dog".into()));
        assert_eq!(state.tabs.recommend.filtered.len(), 1);
    }
}

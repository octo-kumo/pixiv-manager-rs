//! Parsing of browser-exported Netscape `cookies.txt` files.
//!
//! The app only needs the session pair (`auth_token` + `ct0`, which doubles
//! as the `x-csrf-token` header) plus the account id in `twid`; every cookie
//! is forwarded verbatim in the `Cookie` header so Cloudflare and session
//! cookies keep working.
//!
//! Secret values are never logged and the [`Debug`] impl redacts them.

use super::error::TwitterError;
use std::path::Path;

/// Credentials extracted from a Netscape cookies file.
#[derive(Clone)]
pub struct TwitterCredentials {
    /// Session cookie (`auth_token`).
    pub auth_token: String,
    /// CSRF token (`ct0`), also sent as `x-csrf-token`.
    pub ct0: String,
    /// The logged-in account's numeric id (from `twid`).
    pub user_id: u64,
    /// All cookies as a single `name=value; …` header value.
    pub cookie_header: String,
}

impl std::fmt::Debug for TwitterCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TwitterCredentials")
            .field("user_id", &self.user_id)
            .field("auth_token", &"<redacted>")
            .field("ct0", &"<redacted>")
            .field("cookies", &self.cookie_header.split("; ").count())
            .finish()
    }
}

impl TwitterCredentials {
    /// Whether the session cookies are present.
    pub fn is_complete(&self) -> bool {
        !self.auth_token.is_empty() && !self.ct0.is_empty() && self.user_id != 0
    }
}

/// Read + parse a Netscape cookies file from disk.
pub fn load_cookies_file(path: &Path) -> Result<TwitterCredentials, TwitterError> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| TwitterError::Cookies(format!("cannot read {}: {e}", path.display())))?;
    parse_netscape(&text)
}

/// Parse Netscape-format cookie text.
///
/// Each non-comment line is 7 tab/whitespace separated fields:
/// `domain  include_subdomains  path  secure  expires  name  value`.
/// Some exporters use spaces instead of tabs; both are accepted.
pub fn parse_netscape(text: &str) -> Result<TwitterCredentials, TwitterError> {
    let mut auth_token: Option<String> = None;
    let mut ct0: Option<String> = None;
    let mut twid: Option<String> = None;
    let mut pairs: Vec<String> = Vec::new();

    for line in text.lines() {
        // Some exporters mark HttpOnly cookies with `#HttpOnly_` (curl's jar
        // format). Those lines are real cookies — strip the marker; every
        // other `#` line is a comment.
        let line = match line.trim().strip_prefix("#HttpOnly_") {
            Some(rest) => rest.trim(),
            None => {
                let trimmed = line.trim();
                if trimmed.is_empty() || trimmed.starts_with('#') {
                    continue;
                }
                trimmed
            }
        };
        let fields: Vec<&str> = if line.contains('\t') {
            line.split('\t').collect()
        } else {
            line.split_whitespace().collect()
        };
        if fields.len() < 7 {
            continue;
        }
        let name = fields[5];
        let value = fields[6].trim_matches('"');
        match name {
            "auth_token" => auth_token = Some(value.to_string()),
            "ct0" => ct0 = Some(value.to_string()),
            "twid" => twid = Some(value.to_string()),
            _ => {}
        }
        pairs.push(format!("{name}={value}"));
    }

    let auth_token =
        auth_token.ok_or_else(|| TwitterError::Cookies("missing auth_token cookie".into()))?;
    let ct0 = ct0.ok_or_else(|| TwitterError::Cookies("missing ct0 cookie".into()))?;
    let user_id = twid
        .as_deref()
        .and_then(parse_twid)
        .ok_or_else(|| TwitterError::Cookies("missing or invalid twid cookie".into()))?;

    Ok(TwitterCredentials {
        auth_token,
        ct0,
        user_id,
        cookie_header: pairs.join("; "),
    })
}

/// `twid` is stored as `u%3D<id>` (URL-encoded `u=<id>`); tolerate the
/// decoded and bare forms too.
fn parse_twid(value: &str) -> Option<u64> {
    let decoded = value.replace("%3D", "=").replace("%3d", "=");
    let id = decoded.rsplit('=').next().unwrap_or(decoded.as_str());
    id.trim().parse::<u64>().ok().filter(|id| *id != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
# Netscape HTTP Cookie File
.x.com\tTRUE\t/\tTRUE\t1811037380267\tguest_id\tv1%3A177647738023510461
.x.com\tTRUE\t/\tTRUE\t1811037413901\tauth_token\tdeadbeef
.x.com\tTRUE\t/\tTRUE\t1811037414108\tct0\tcafef00d
.x.com\tTRUE\t/\tTRUE\t1821396176485\ttwid\tu%3D981068766313197568
";

    #[test]
    fn parses_session_cookies() {
        let c = parse_netscape(SAMPLE).expect("parses");
        assert_eq!(c.auth_token, "deadbeef");
        assert_eq!(c.ct0, "cafef00d");
        assert_eq!(c.user_id, 981068766313197568);
        assert!(c.is_complete());
        assert!(c.cookie_header.contains("auth_token=deadbeef"));
        assert!(c.cookie_header.contains("guest_id=v1%3A"));
    }

    #[test]
    fn accepts_decoded_twid_and_spaces() {
        let text = "x.com TRUE / TRUE 1 auth_token a\n\
                    x.com TRUE / TRUE 1 ct0 b\n\
                    x.com TRUE / TRUE 1 twid u=42\n";
        let c = parse_netscape(text).expect("parses");
        assert_eq!(c.user_id, 42);
    }

    #[test]
    fn accepts_httponly_marker_lines() {
        let text = "#HttpOnly_.x.com\tTRUE\t/\tTRUE\t1\tauth_token\ta\n\
                    #HttpOnly_.x.com\tTRUE\t/\tTRUE\t1\tct0\tb\n\
                    .x.com\tTRUE\t/\tTRUE\t1\ttwid\tu%3D7\n\
                    # a real comment\n";
        let c = parse_netscape(text).expect("parses");
        assert_eq!(c.auth_token, "a");
        assert_eq!(c.ct0, "b");
        assert_eq!(c.user_id, 7);
    }

    #[test]
    fn missing_required_cookie_is_an_error() {
        let text = ".x.com\tTRUE\t/\tTRUE\t1\tauth_token\ta\n";
        let err = parse_netscape(text).expect_err("must fail");
        assert!(matches!(err, TwitterError::Cookies(_)));
    }

    #[test]
    fn debug_redacts_secrets() {
        let c = parse_netscape(SAMPLE).unwrap();
        let dbg = format!("{:?}", c);
        assert!(!dbg.contains("deadbeef"));
        assert!(!dbg.contains("cafef00d"));
        assert!(dbg.contains("981068766313197568"));
    }
}

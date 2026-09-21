//! Parsing of browser-exported Netscape `cookies.txt` files for Bilibili.
//!
//! The app needs `SESSDATA` (the session), `DedeUserID` (the account id) and
//! `bili_jct` (the CSRF token for writes — likes). Every cookie is forwarded
//! verbatim so `buvid*`, `bili_ticket` and risk-control cookies keep working.
//!
//! Secret values are never logged and the [`Debug`] impl redacts them.

use super::error::BilibiliError;
use std::path::Path;

/// Credentials extracted from a Netscape cookies file.
#[derive(Clone)]
pub struct BilibiliCredentials {
    /// Session cookie (`SESSDATA`).
    pub sessdata: String,
    /// CSRF token (`bili_jct`); empty when the export lacked it (read-only).
    pub bili_jct: String,
    /// The logged-in account's uid (from `DedeUserID`).
    pub user_id: u64,
    /// All cookies as a single `name=value; …` header value.
    pub cookie_header: String,
}

impl std::fmt::Debug for BilibiliCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BilibiliCredentials")
            .field("user_id", &self.user_id)
            .field("sessdata", &"<redacted>")
            .field("bili_jct", &(!self.bili_jct.is_empty()))
            .field("cookies", &self.cookie_header.split("; ").count())
            .finish()
    }
}

impl BilibiliCredentials {
    /// Whether the session cookie + account id are present.
    pub fn is_complete(&self) -> bool {
        !self.sessdata.is_empty() && self.user_id != 0
    }

    /// Whether write actions (likes) are possible.
    pub fn can_write(&self) -> bool {
        self.is_complete() && !self.bili_jct.is_empty()
    }
}

/// Read + parse a Netscape cookies file from disk.
pub fn load_cookies_file(path: &Path) -> Result<BilibiliCredentials, BilibiliError> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| BilibiliError::Cookies(format!("cannot read {}: {e}", path.display())))?;
    parse_netscape(&text)
}

/// Parse Netscape-format cookie text.
///
/// Each non-comment line is 7 tab/whitespace separated fields:
/// `domain  include_subdomains  path  secure  expires  name  value`.
/// `#HttpOnly_`-prefixed lines (common in exported jars) are parsed too.
pub fn parse_netscape(text: &str) -> Result<BilibiliCredentials, BilibiliError> {
    let mut sessdata: Option<String> = None;
    let mut bili_jct: Option<String> = None;
    let mut dede_user_id: Option<String> = None;
    let mut pairs: Vec<String> = Vec::new();

    for line in text.lines() {
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
            "SESSDATA" => sessdata = Some(value.to_string()),
            "bili_jct" => bili_jct = Some(value.to_string()),
            "DedeUserID" => dede_user_id = Some(value.to_string()),
            _ => {}
        }
        pairs.push(format!("{name}={value}"));
    }

    let sessdata =
        sessdata.ok_or_else(|| BilibiliError::Cookies("missing SESSDATA cookie".into()))?;
    let user_id = dede_user_id
        .as_deref()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|id| *id != 0)
        .ok_or_else(|| BilibiliError::Cookies("missing or invalid DedeUserID cookie".into()))?;

    Ok(BilibiliCredentials {
        sessdata,
        bili_jct: bili_jct.unwrap_or_default(),
        user_id,
        cookie_header: pairs.join("; "),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
# Netscape HTTP Cookie File
.bilibili.com\tTRUE\t/\tTRUE\t1811037380267\tbuvid3\tabc
.bilibili.com\tTRUE\t/\tTRUE\t1811037413901\tSESSDATA\tdeadbeef%2C123
.bilibili.com\tTRUE\t/\tTRUE\t1811037414108\tbili_jct\tcafef00d
.bilibili.com\tTRUE\t/\tTRUE\t1821396176485\tDedeUserID\t522541093
";

    #[test]
    fn parses_account_cookies() {
        let c = parse_netscape(SAMPLE).expect("parses");
        assert_eq!(c.sessdata, "deadbeef%2C123");
        assert_eq!(c.bili_jct, "cafef00d");
        assert_eq!(c.user_id, 522541093);
        assert!(c.is_complete());
        assert!(c.can_write());
        assert!(c.cookie_header.contains("SESSDATA=deadbeef%2C123"));
    }

    #[test]
    fn accepts_httponly_and_missing_csrf() {
        let text = "#HttpOnly_.bilibili.com\tTRUE\t/\tTRUE\t1\tSESSDATA\ts\n\
                    .bilibili.com\tTRUE\t/\tTRUE\t1\tDedeUserID\t42\n";
        let c = parse_netscape(text).expect("parses");
        assert_eq!(c.user_id, 42);
        assert!(!c.can_write(), "no bili_jct means read-only");
    }

    #[test]
    fn missing_session_is_an_error() {
        let text = ".bilibili.com\tTRUE\t/\tTRUE\t1\tDedeUserID\t42\n";
        let err = parse_netscape(text).expect_err("must fail");
        assert!(matches!(err, BilibiliError::Cookies(_)));
    }

    #[test]
    fn debug_redacts_secrets() {
        let c = parse_netscape(SAMPLE).unwrap();
        let dbg = format!("{:?}", c);
        assert!(!dbg.contains("deadbeef"));
        assert!(!dbg.contains("cafef00d"));
        assert!(dbg.contains("522541093"));
    }
}

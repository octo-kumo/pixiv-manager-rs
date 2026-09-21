//! WBI request signing.
//!
//! Bilibili's web APIs expect `wts` (a timestamp) plus a `w_rid` signature
//! derived from the sorted, encoded parameters and a per-session "mixin key".
//! The mixin key is built by permuting the concatenated `img_key`/`sub_key`
//! from `x/web-interface/nav` through a fixed index table (the same table the
//! web client uses).

use md5::{Digest, Md5};

/// The fixed permutation table (Bilibili web client).
const MIXIN_KEY_ENC_TAB: [usize; 64] = [
    46, 47, 18, 2, 53, 8, 23, 32, 15, 50, 10, 31, 58, 3, 45, 35, 27, 43, 5, 49, 33, 9, 42, 19, 29,
    28, 14, 39, 12, 38, 41, 13, 37, 48, 7, 16, 24, 55, 40, 61, 26, 17, 0, 1, 60, 51, 30, 4, 22, 25,
    54, 21, 56, 59, 6, 63, 57, 62, 11, 36, 20, 34, 44, 52,
];

/// The per-session WBI keys from `nav`.
#[derive(Debug, Clone)]
pub struct WbiKeys {
    pub img_key: String,
    pub sub_key: String,
}

impl WbiKeys {
    /// Build from the `wbi_img.img_url` / `wbi_img.sub_url` values (their
    /// file stems are the keys).
    pub fn from_urls(img_url: &str, sub_url: &str) -> Self {
        Self {
            img_key: file_stem(img_url),
            sub_key: file_stem(sub_url),
        }
    }

    /// The 32-char mixin key used to sign requests.
    pub fn mixin_key(&self) -> String {
        let raw = format!("{}{}", self.img_key, self.sub_key);
        let permuted: String = MIXIN_KEY_ENC_TAB
            .iter()
            .filter_map(|i| raw.as_bytes().get(*i))
            .map(|b| *b as char)
            .collect();
        permuted.chars().take(32).collect()
    }
}

fn file_stem(url: &str) -> String {
    url.rsplit('/')
        .next()
        .unwrap_or(url)
        .split('.')
        .next()
        .unwrap_or("")
        .to_string()
}

/// Sign query parameters: appends `wts` and `w_rid` to the percent-encoded,
/// key-sorted parameter string. `chars` in `!'()*` are stripped from values
/// before signing, matching the web client.
pub fn sign(params: &[(String, String)], mixin_key: &str) -> String {
    let wts = chrono::Utc::now().timestamp();
    let mut all: Vec<(String, String)> = params.to_vec();
    all.push(("wts".to_string(), wts.to_string()));
    all.sort_by(|a, b| a.0.cmp(&b.0));
    let query: Vec<String> = all
        .iter()
        .map(|(k, v)| {
            let clean: String = v.chars().filter(|c| !"!'()*".contains(*c)).collect();
            format!("{}={}", urlencode(k), urlencode(&clean))
        })
        .collect();
    let query = query.join("&");
    let mut hasher = Md5::new();
    hasher.update(format!("{query}{mixin_key}").as_bytes());
    let digest = hasher.finalize();
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    format!("{query}&w_rid={hex}")
}

/// Minimal percent-encoding for query keys/values (RFC 3986 unreserved set).
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixin_key_matches_known_algorithm() {
        // From the live nav response, this pair produces a stable 32-char key.
        let keys = WbiKeys::from_urls(
            "https://i0.hdslb.com/bfs/wbi/7cd084941338484aae1ad9425b84077c.png",
            "https://i0.hdslb.com/bfs/wbi/4932caff0ff746eab6f01bf08b70ac45.png",
        );
        assert_eq!(keys.img_key, "7cd084941338484aae1ad9425b84077c");
        assert_eq!(keys.sub_key, "4932caff0ff746eab6f01bf08b70ac45");
        let mixin = keys.mixin_key();
        assert_eq!(mixin.len(), 32);
        // Deterministic for the same keys.
        assert_eq!(mixin, keys.mixin_key());
    }

    #[test]
    fn sign_is_deterministic_shape() {
        let keys = WbiKeys::from_urls("https://x/a.png", "https://x/b.png");
        let q = sign(
            &[
                ("oid".to_string(), "123".to_string()),
                ("type".to_string(), "11".to_string()),
            ],
            &keys.mixin_key(),
        );
        assert!(q.contains("oid=123"));
        assert!(q.contains("type=11"));
        assert!(q.contains("wts="));
        assert!(q.contains("w_rid="));
        // Sorted by key: oid before type before wts.
        let oid = q.find("oid=").unwrap();
        let ty = q.find("type=").unwrap();
        let wts = q.find("wts=").unwrap();
        assert!(oid < ty && ty < wts);
    }

    #[test]
    fn values_strip_reserved_chars() {
        let keys = WbiKeys::from_urls("https://x/a.png", "https://x/b.png");
        let q = sign(
            &[("q".to_string(), "a!b'c(d)e*f".to_string())],
            &keys.mixin_key(),
        );
        assert!(q.contains("q=abcdef"), "got {q}");
    }
}

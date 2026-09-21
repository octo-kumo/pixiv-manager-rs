//! Per-source behavior: the small provider-specific decisions (image headers,
//! public web URLs, badges, download-directory prefixes) that used to be
//! re-derived as scattered `match Provider` blocks across the app.
//!
//! All of it hangs off [`crate::media::Provider`] as methods, so generic code
//! asks the provider instead of branching on it. Network clients stay in the
//! provider modules ([`crate::twitter`], [`crate::bilibili`]); only their
//! externally-required constants surface here.

use crate::media::{native_id, Provider};
use pixivj_rs::model::Illustration;

/// The browser User-Agent Twitter's GraphQL and Bilibili's risk control
/// require (both reject library-shaped clients outright).
pub const BROWSER_USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

/// User-Agent impersonating the Pixiv Android app (required by the API/CDN).
pub const PIXIV_APP_USER_AGENT: &str = "PixivAndroidApp/5.0.234 (Android 9.0; Pixel 3)";

/// Referer Pixiv's image CDN requires (i.pximg.net 403s without it).
pub const PIXIV_CDN_REFERER: &str = "https://app-api.pixiv.net/";

/// Referer Bilibili's image CDN expects.
pub const BILIBILI_REFERER: &str = "https://www.bilibili.com/";

impl Provider {
    /// Short badge shown on gallery tiles for non-Pixiv items.
    pub fn badge(self) -> Option<&'static str> {
        match self {
            Provider::Pixiv => None,
            Provider::Twitter => Some("𝕏"),
            Provider::Bilibili => Some("B"),
        }
    }

    /// Download-directory name prefix (`""` for Pixiv, `"tw_"`, `"bl_"`).
    pub fn dir_prefix(self) -> &'static str {
        match self {
            Provider::Pixiv => "",
            Provider::Twitter => "tw_",
            Provider::Bilibili => "bl_",
        }
    }

    /// `(User-Agent, Referer)` for image-CDN requests. Pixiv requires both;
    /// Twitter's CDN must not receive a foreign Referer.
    pub fn image_headers(self) -> (&'static str, Option<&'static str>) {
        match self {
            Provider::Pixiv => (PIXIV_APP_USER_AGENT, Some(PIXIV_CDN_REFERER)),
            Provider::Twitter => (BROWSER_USER_AGENT, None),
            Provider::Bilibili => (BROWSER_USER_AGENT, Some(BILIBILI_REFERER)),
        }
    }

    /// Tooltip for the "copy URL" action.
    pub fn copy_url_tip(self) -> &'static str {
        match self {
            Provider::Pixiv => "Copy pixiv URL",
            Provider::Twitter => "Copy tweet URL",
            Provider::Bilibili => "Copy post URL",
        }
    }

    /// The public web URL of an item. `illust` (when it is the item being
    /// linked) lets Twitter use the real handle-qualified URL.
    pub fn web_url(self, illust: Option<&Illustration>, native: u64) -> String {
        match self {
            Provider::Pixiv => format!("https://www.pixiv.net/en/artworks/{native}"),
            Provider::Twitter => illust
                .and_then(crate::twitter::convert::tweet_web_url)
                .unwrap_or_else(|| format!("https://x.com/i/status/{native}")),
            Provider::Bilibili => format!("https://t.bilibili.com/{native}"),
        }
    }

    /// The public web URL of a media item by namespaced id.
    pub fn web_url_of(self, illust: Option<&Illustration>, id: u64) -> String {
        self.web_url(illust.filter(|i| i.id == id), native_id(id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dir_prefixes_match_core_layout() {
        assert_eq!(Provider::Pixiv.dir_prefix(), "");
        assert_eq!(Provider::Twitter.dir_prefix(), "tw_");
        assert_eq!(Provider::Bilibili.dir_prefix(), "bl_");
    }

    #[test]
    fn only_pixiv_has_no_badge() {
        assert_eq!(Provider::Pixiv.badge(), None);
        assert_eq!(Provider::Twitter.badge(), Some("𝕏"));
        assert_eq!(Provider::Bilibili.badge(), Some("B"));
    }

    #[test]
    fn web_urls_are_provider_specific() {
        assert_eq!(
            Provider::Pixiv.web_url(None, 123),
            "https://www.pixiv.net/en/artworks/123"
        );
        assert_eq!(
            Provider::Twitter.web_url(None, 456),
            "https://x.com/i/status/456"
        );
        assert_eq!(
            Provider::Bilibili.web_url(None, 789),
            "https://t.bilibili.com/789"
        );
    }
}

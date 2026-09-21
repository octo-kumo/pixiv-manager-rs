//! Provider identity for the app's unified media model.
//!
//! Every item shown, opened, cached or persisted (gallery, viewer, library,
//! download queue) is a `pixivj_rs::model::Illustration`. Non-Pixiv providers
//! convert their posts into that same shape, and their native ids are
//! namespaced with a provider bit so every `u64`-keyed cache
//! (`thumbnail_handles`, `palettes`, `downloaded`, the download queue, the
//! bookmark library, …) keeps working across mixed lists without collisions.
//!
//! Bit allocation (native ids must stay below 2^62, which holds for Pixiv
//! illust ids, Twitter snowflakes until ~2046 and Bilibili dynamic ids):
//!   bit 63 = Twitter, bit 62 = Bilibili, neither = Pixiv.

/// Twitter's provider bit (kept at bit 63 for backward compatibility with
/// already-persisted data).
pub const PROVIDER_BIT: u64 = 1 << 63;
/// Bilibili's provider bit.
pub const BILIBILI_BIT: u64 = 1 << 62;
/// Mask of every provider bit.
pub const PROVIDER_MASK: u64 = PROVIDER_BIT | BILIBILI_BIT;

/// The provider a media item (or provider-owned id, e.g. an author id) comes
/// from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Provider {
    #[default]
    Pixiv,
    Twitter,
    Bilibili,
}

impl Provider {
    /// The provider that owns a namespaced id.
    pub fn of(id: u64) -> Self {
        if id & PROVIDER_BIT != 0 {
            Provider::Twitter
        } else if id & BILIBILI_BIT != 0 {
            Provider::Bilibili
        } else {
            Provider::Pixiv
        }
    }

    /// Short display label (used by provider badges / labels).
    pub fn label(self) -> &'static str {
        match self {
            Provider::Pixiv => "Pixiv",
            Provider::Twitter => "Twitter",
            Provider::Bilibili => "Bilibili",
        }
    }

    /// Namespace a provider-native id into the app's global id space.
    pub fn key(self, native: u64) -> u64 {
        match self {
            Provider::Pixiv => native,
            Provider::Twitter => twitter_key(native),
            Provider::Bilibili => bilibili_key(native),
        }
    }
}

/// The provider that owns a namespaced id.
pub fn provider_of(id: u64) -> Provider {
    Provider::of(id)
}

/// Whether `id` belongs to the Twitter provider.
pub fn is_twitter(id: u64) -> bool {
    id & PROVIDER_BIT != 0
}

/// Whether `id` belongs to the Bilibili provider.
pub fn is_bilibili(id: u64) -> bool {
    id & BILIBILI_BIT != 0
}

/// Whether `id` belongs to Pixiv.
pub fn is_pixiv(id: u64) -> bool {
    id & PROVIDER_MASK == 0
}

/// Namespace a native tweet (or tweet-author) id.
pub fn twitter_key(native: u64) -> u64 {
    PROVIDER_BIT | native
}

/// Namespace a native Bilibili dynamic (or user) id.
pub fn bilibili_key(native: u64) -> u64 {
    BILIBILI_BIT | native
}

/// Strip the provider namespace, yielding the provider-native id.
pub fn native_id(id: u64) -> u64 {
    id & !PROVIDER_MASK
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_roundtrip() {
        assert_eq!(provider_of(123), Provider::Pixiv);
        assert_eq!(native_id(123), 123);
        let key = twitter_key(123);
        assert_eq!(provider_of(key), Provider::Twitter);
        assert_eq!(native_id(key), 123);
        assert!(is_twitter(key));
        assert!(!is_twitter(123));
        let bl = bilibili_key(456);
        assert_eq!(provider_of(bl), Provider::Bilibili);
        assert_eq!(native_id(bl), 456);
        assert!(is_bilibili(bl));
        assert!(!is_twitter(bl));
        assert!(is_pixiv(123));
        assert!(!is_pixiv(bl));
        // Realistic snowflake / dynamic ids stay below the provider bits.
        let snowflake = 1_900_000_000_000_000_000u64;
        assert!(snowflake < BILIBILI_BIT);
        assert_eq!(native_id(twitter_key(snowflake)), snowflake);
    }

    #[test]
    fn provider_key_helpers_agree() {
        assert_eq!(Provider::Pixiv.key(42), 42);
        assert_eq!(Provider::Twitter.key(42), twitter_key(42));
        assert_eq!(Provider::Bilibili.key(42), bilibili_key(42));
        assert_eq!(Provider::of(42).label(), "Pixiv");
        assert_eq!(Provider::of(twitter_key(42)).label(), "Twitter");
        assert_eq!(Provider::of(bilibili_key(42)).label(), "Bilibili");
    }
}

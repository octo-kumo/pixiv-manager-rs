//! Emoji font discovery.
//!
//! iced only renders fonts it has loaded, and the color emoji face is not part
//! of the default set. Each platform keeps it in a different place; when none
//! is found the app simply renders emoji as boxes.

use std::path::PathBuf;

/// Emoji font files to try, most preferred first.
pub fn emoji_font_candidates() -> Vec<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        let root = std::env::var_os("SystemRoot")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
        vec![root.join("Fonts").join("seguiemj.ttf")]
    }
    #[cfg(target_os = "macos")]
    {
        vec![PathBuf::from("/System/Library/Fonts/Apple Color Emoji.ttc")]
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        const PATHS: &[&str] = &[
            "/usr/share/fonts/google-noto-color-emoji-fonts/Noto-COLRv1.ttf",
            "/usr/share/fonts/truetype/noto/NotoColorEmoji.ttf",
            "/usr/share/fonts/opentype/noto/NotoColorEmoji.ttf",
            "/usr/share/fonts/noto/NotoColorEmoji.ttf",
        ];
        PATHS.iter().map(PathBuf::from).collect()
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", unix)))]
    {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offers_at_least_one_platform_appropriate_candidate() {
        let candidates = emoji_font_candidates();
        #[cfg(any(target_os = "windows", target_os = "macos", unix))]
        assert!(!candidates.is_empty());
        for path in candidates {
            assert!(
                path.extension().is_some(),
                "candidate must be a font file: {path:?}"
            );
        }
    }
}

//! The platform's primary shortcut modifier and the labels derived from it.
//!
//! Standard application shortcuts use Control on Linux/Windows and Command
//! (⌘) on macOS. Application code asks [`primary_modifier`] instead of
//! testing `modifiers.control()`, and renders hints through
//! [`primary_shortcut`] instead of hard-coding `"Ctrl+F"`.

use iced::keyboard::Modifiers;

/// Is the platform's primary shortcut modifier held?
///
/// * Linux / Windows — Control
/// * macOS — Command (Logo)
///
/// Deliberately *not* "either Control or Command" on macOS: binding both would
/// steal Control-based readline/terminal-style bindings and surprises users.
pub fn primary_modifier(modifiers: Modifiers) -> bool {
    #[cfg(target_os = "macos")]
    {
        modifiers.logo()
    }
    #[cfg(not(target_os = "macos"))]
    {
        modifiers.control()
    }
}

/// The primary modifier as shown in shortcut hints: `Ctrl` or `⌘`.
pub const fn primary_label() -> &'static str {
    if cfg!(target_os = "macos") {
        "⌘"
    } else {
        "Ctrl"
    }
}

/// Render `primary modifier + key` the way each platform writes it:
/// `Ctrl+F` / `Ctrl+-` on Linux and Windows, `⌘F` / `⌘-` on macOS.
pub fn primary_shortcut(key: &str) -> String {
    if cfg!(target_os = "macos") {
        format!("{}{}", primary_label(), key)
    } else {
        format!("{}+{}", primary_label(), key)
    }
}

/// The key cap for "minus" on this platform: the typographic minus reads
/// better next to ⌘, and `-` matches what users type elsewhere.
pub const fn minus_label() -> &'static str {
    if cfg!(target_os = "macos") {
        "−"
    } else {
        "-"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_is_primary_off_macos() {
        let modifiers = Modifiers::CTRL;
        #[cfg(not(target_os = "macos"))]
        assert!(primary_modifier(modifiers));
        #[cfg(target_os = "macos")]
        assert!(!primary_modifier(modifiers));
    }

    #[test]
    fn logo_is_primary_on_macos_only() {
        let modifiers = Modifiers::LOGO;
        #[cfg(target_os = "macos")]
        assert!(primary_modifier(modifiers));
        #[cfg(not(target_os = "macos"))]
        assert!(!primary_modifier(modifiers));
    }

    #[test]
    fn no_modifiers_is_never_primary() {
        assert!(!primary_modifier(Modifiers::NONE));
        assert!(!primary_modifier(Modifiers::SHIFT | Modifiers::ALT));
    }

    #[test]
    fn labels_match_the_platform() {
        if cfg!(target_os = "macos") {
            assert_eq!(primary_label(), "⌘");
            assert_eq!(primary_shortcut("F"), "⌘F");
            assert_eq!(primary_shortcut(minus_label()), "⌘−");
        } else {
            assert_eq!(primary_label(), "Ctrl");
            assert_eq!(primary_shortcut("F"), "Ctrl+F");
            assert_eq!(primary_shortcut(minus_label()), "Ctrl+-");
        }
    }
}

//! The platform's primary shortcut modifier and its label.

use iced::keyboard::Modifiers;

/// Control on Linux/Windows, Command on macOS.
///
/// Deliberately not "Ctrl or Command" on macOS: binding both would shadow
/// Control-based bindings the platform reserves for text editing.
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

pub const fn primary_label() -> &'static str {
    if cfg!(target_os = "macos") {
        "⌘"
    } else {
        "Ctrl"
    }
}

/// `Ctrl+F` on Linux/Windows, `⌘F` on macOS.
pub fn primary_shortcut(key: &str) -> String {
    if cfg!(target_os = "macos") {
        format!("{}{}", primary_label(), key)
    } else {
        format!("{}+{}", primary_label(), key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primary_modifier_is_ctrl_off_macos() {
        #[cfg(not(target_os = "macos"))]
        assert!(primary_modifier(Modifiers::CTRL));
        #[cfg(target_os = "macos")]
        assert!(!primary_modifier(Modifiers::CTRL));
    }

    #[test]
    fn primary_modifier_is_command_on_macos() {
        #[cfg(target_os = "macos")]
        assert!(primary_modifier(Modifiers::LOGO));
        #[cfg(not(target_os = "macos"))]
        assert!(!primary_modifier(Modifiers::LOGO));
    }

    #[test]
    fn no_modifiers_is_never_primary() {
        assert!(!primary_modifier(Modifiers::NONE));
        assert!(!primary_modifier(Modifiers::SHIFT | Modifiers::ALT));
    }

    #[test]
    fn labels_match_the_platform() {
        if cfg!(target_os = "macos") {
            assert_eq!(primary_shortcut("F"), "⌘F");
            assert_eq!(primary_shortcut("-"), "⌘-");
        } else {
            assert_eq!(primary_shortcut("F"), "Ctrl+F");
            assert_eq!(primary_shortcut("-"), "Ctrl+-");
        }
    }
}

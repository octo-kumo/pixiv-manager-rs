use crate::app::message::Message;
use crate::app::state::AppState;
use iced::futures::StreamExt;

/// Application subscriptions.
///
/// * Captures the runtime window id (needed by `iced::window::close`).
/// * Tracks window resize events so the gallery/viewer can use real size.
/// * Detects the window being closed.
/// * Follows OS dark/light theme changes.
/// * While the viewer is open, Esc closes it.
/// * While onboarding, polls the token channel (never blocks, so the app can
///   always be closed cleanly).
pub fn subscription(state: &AppState) -> iced::Subscription<Message> {
    let mut subs: Vec<iced::Subscription<Message>> = vec![
        iced::event::listen_with(|event, _status, window| match event {
            iced::Event::Window(iced::window::Event::Opened { .. }) => {
                Some(Message::WindowRegistered(window))
            }
            iced::Event::Window(iced::window::Event::Resized(size)) => {
                Some(Message::WindowResized {
                    width: size.width,
                    height: size.height,
                })
            }
            iced::Event::Window(iced::window::Event::Rescaled(scale)) => {
                Some(Message::ScaleFactorChanged(scale))
            }
            iced::Event::Window(iced::window::Event::Closed) => Some(Message::WindowClosed),
            _ => None,
        }),
        iced::system::theme_changes().map(Message::SystemThemeChanged),
    ];

    // Track Shift so a gallery click can toggle a bookmark instead of opening
    // the viewer (shift-click). The closure captures nothing.
    subs.push(iced::event::listen_with(
        |event, _status, _window| match event {
            iced::Event::Keyboard(iced::keyboard::Event::ModifiersChanged(modifiers)) => {
                Some(Message::ModifiersChanged(modifiers.shift()))
            }
            _ => None,
        },
    ));

    // The OS close button: update() decides whether to exit the overlay
    // (viewer/artist) or close the app, so this closure captures nothing.
    subs.push(iced::event::listen_with(
        |event, _status, window| match event {
            iced::Event::Window(iced::window::Event::CloseRequested) => {
                Some(Message::WindowCloseRequested(window))
            }
            _ => None,
        },
    ));
    // Esc closes the topmost overlay (viewer, artist page, settings, help
    // or shortcuts screen).
    // The closure must not capture anything (Subscription::map), so it emits
    // a generic EscapePressed and update() decides what to close.
    if state.viewer.open
        || state.artist.open
        || state.settings_open
        || state.show_help
        || state.show_shortcuts
    {
        subs.push(
            iced::keyboard::listen()
                .map(|event| match event {
                    iced::keyboard::Event::KeyPressed {
                        key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape),
                        ..
                    } => Some(Message::EscapePressed),
                    _ => None,
                })
                .filter_map(|msg| msg),
        );
    }

    // Browser-style back/forward page shortcuts in the viewer: Alt+← / Alt+→
    // step through a multi-page illust.
    if state.viewer.open {
        subs.push(iced::event::listen_with(|event, _status, _window| {
            let iced::Event::Keyboard(iced::keyboard::Event::KeyPressed { key, modifiers, .. }) =
                event
            else {
                return None;
            };
            if !modifiers.alt() {
                return None;
            }
            match key {
                iced::keyboard::Key::Named(iced::keyboard::key::Named::ArrowLeft) => {
                    Some(Message::ViewerPageStep(-1))
                }
                iced::keyboard::Key::Named(iced::keyboard::key::Named::ArrowRight) => {
                    Some(Message::ViewerPageStep(1))
                }
                _ => None,
            }
        }));
    }

    // Viewer page navigation: ←/→ flip pages, Home/End jump to the
    // first/last one. Only subscribed while a multi-page work is open.
    if state.viewer.open && state.viewer.total_pages > 1 {
        use iced::keyboard::key::Named;
        subs.push(
            iced::keyboard::listen()
                .map(|event| match event {
                    iced::keyboard::Event::KeyPressed {
                        key: iced::keyboard::Key::Named(Named::ArrowLeft),
                        ..
                    } => Some(Message::ViewerPageNavigate(
                        crate::app::state::ViewerPageNav::Prev,
                    )),
                    iced::keyboard::Event::KeyPressed {
                        key: iced::keyboard::Key::Named(Named::ArrowRight),
                        ..
                    } => Some(Message::ViewerPageNavigate(
                        crate::app::state::ViewerPageNav::Next,
                    )),
                    iced::keyboard::Event::KeyPressed {
                        key: iced::keyboard::Key::Named(Named::Home),
                        ..
                    } => Some(Message::ViewerPageNavigate(
                        crate::app::state::ViewerPageNav::First,
                    )),
                    iced::keyboard::Event::KeyPressed {
                        key: iced::keyboard::Key::Named(Named::End),
                        ..
                    } => Some(Message::ViewerPageNavigate(
                        crate::app::state::ViewerPageNav::Last,
                    )),
                    _ => None,
                })
                .filter_map(|msg| msg),
        );
    }

    // F3 toggles the floating debug overlay.
    if state.phase == crate::app::state::AppPhase::Main {
        subs.push(
            iced::keyboard::listen()
                .map(|event| match event {
                    iced::keyboard::Event::KeyPressed {
                        key: iced::keyboard::Key::Named(iced::keyboard::key::Named::F3),
                        ..
                    } => Some(Message::ToggleDebugOverlay),
                    _ => None,
                })
                .filter_map(|msg| msg),
        );
    }

    // F1 toggles the keyboard-shortcuts overlay (main phase only; F1 is never
    // typed into a text field, unlike '?').
    if state.phase == crate::app::state::AppPhase::Main {
        subs.push(
            iced::keyboard::listen()
                .map(|event| match event {
                    iced::keyboard::Event::KeyPressed {
                        key: iced::keyboard::Key::Named(iced::keyboard::key::Named::F1),
                        ..
                    } => Some(Message::ToggleShortcuts),
                    _ => None,
                })
                .filter_map(|msg| msg),
        );
    }

    // Ctrl+F focuses the search input (main phase, no overlay on top).
    if state.phase == crate::app::state::AppPhase::Main
        && !state.viewer.open
        && !state.artist.open
        && !state.settings_open
        && !state.show_help
    {
        subs.push(iced::event::listen_with(|event, _status, _window| {
            let iced::Event::Keyboard(iced::keyboard::Event::KeyPressed { key, modifiers, .. }) =
                event
            else {
                return None;
            };
            match (key, modifiers.control()) {
                (iced::keyboard::Key::Character(c), true) if c == "f" => Some(Message::FocusSearch),
                _ => None,
            }
        }));
    }

    // Autocomplete ↑/↓ while suggestions are visible on the Search tab.
    // Only vertical arrows are intercepted — in a single-line input they do
    // nothing to the caret, so there's no conflict.
    if state.phase == crate::app::state::AppPhase::Main
        && !state.viewer.open
        && !state.artist.open
        && !state.settings_open
        && state.active_tab == crate::app::state::Tab::Search
        && !state.autocomplete_tags.is_empty()
    {
        subs.push(iced::event::listen_with(|event, _status, _window| {
            let iced::Event::Keyboard(iced::keyboard::Event::KeyPressed { key, modifiers, .. }) =
                event
            else {
                return None;
            };
            if modifiers.control() || modifiers.shift() || modifiers.alt() || modifiers.logo() {
                return None;
            }
            match key {
                iced::keyboard::Key::Named(iced::keyboard::key::Named::ArrowUp) => {
                    Some(Message::AutocompleteMove(-1))
                }
                iced::keyboard::Key::Named(iced::keyboard::key::Named::ArrowDown) => {
                    Some(Message::AutocompleteMove(1))
                }
                _ => None,
            }
        }));
    }

    // Ctrl + / − / 0 adjust the gallery grid size (a common zoom convention).
    subs.push(iced::event::listen_with(|event, _status, _window| {
        let iced::Event::Keyboard(iced::keyboard::Event::KeyPressed { key, modifiers, .. }) = event
        else {
            return None;
        };
        if !modifiers.control() {
            return None;
        }
        match key {
            iced::keyboard::Key::Character(c) if c == "+" || c == "=" => {
                Some(Message::GridSizeStep(10))
            }
            iced::keyboard::Key::Character(c) if c == "-" => Some(Message::GridSizeStep(-10)),
            iced::keyboard::Key::Character(c) if c == "0" => Some(Message::GridSizeStep(0)),
            _ => None,
        }
    }));

    // Onboarding: pick up the submitted token without blocking any thread.
    // `Subscription::run_with` lets us carry the channel receiver as data; the
    // stream just polls `try_recv()` every 100ms, so the UI thread never
    // blocks and the app can always be closed cleanly.
    if let Some(rx) = state.onboarding.rx.clone() {
        subs.push(
            iced::Subscription::run_with(OnboardingRx(rx), |rx| {
                let rx = rx.0.clone();
                iced::futures::stream::unfold(rx, |rx| async move {
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    let token = rx.lock().unwrap().try_recv().ok();
                    Some((token, rx))
                })
                .filter_map(|t| async move { t })
            })
            .map(Message::TokenSubmitted),
        );
    }

    iced::Subscription::batch(subs)
}

/// Hash-able wrapper around the onboarding token receiver so it can be used as
/// [`Subscription::run_with`] data (its identity is a constant tag).
struct OnboardingRx(std::sync::Arc<std::sync::Mutex<std::sync::mpsc::Receiver<String>>>);

impl std::hash::Hash for OnboardingRx {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        "pixiv_manager::onboarding::token_rx".hash(state);
    }
}

use crate::app::message::Message;
use crate::app::state::{AppState, OnboardingStep};
use crate::theme;

pub fn onboarding_view(state: &AppState) -> iced::Element<'_, Message> {
    let t = theme::theme(state);
    let title = iced::widget::text("Pixiv Manager — Setup")
        .size(24)
        .color(theme::accent(&t));

    match &state.onboarding.step {
        OnboardingStep::Starting => {
            let spinner = iced::widget::text("Starting setup server...").size(14);
            iced::widget::column![title, spinner]
                .spacing(16)
                .padding(40)
                .into()
        }

        OnboardingStep::WaitingForToken => {
            let login_url = state.onboarding.login_url.clone().unwrap_or_default();

            // Accent-filled primary button; label color adapts to the theme.
            let login_button = crate::ui::button(
                iced::widget::text("Log in with Pixiv")
                    .size(16)
                    .color(theme::on_accent(&t)),
            )
            .on_press_maybe((!login_url.is_empty()).then_some(Message::OpenUrl(login_url)))
            .padding(iced::Padding::from([10, 24]))
            .style(crate::ui::primary);

            // The last failure (rejected token, failed exchange), if any.
            let error_note: Option<iced::Element<'_, Message>> =
                state.onboarding.error.as_ref().map(|error| {
                    iced::widget::container(iced::widget::text(format!("⚠ {error}")).size(12))
                        .padding(8)
                        .width(iced::Length::Fill)
                        .style(crate::ui::error_banner)
                        .into()
                });

            let mut instructions = iced::widget::column![
                iced::widget::text("To use Pixiv Manager you need to authorize your Pixiv account.")
                    .size(14),
                login_button,
                iced::widget::text("1. Click \"Log in with Pixiv\" above and sign in.").size(12),
                iced::widget::text(
                    "2. The browser will try to open a pixiv://account/login?code=... link and show an error page — that's normal."
                )
                .size(12),
                iced::widget::text(
                    "3. Copy the FULL URL from the address bar (or the code from the F12 Network tab) and paste it below, then click Continue."
                )
                .size(12),
                iced::widget::text("").size(8),
                crate::ui::text_input(
                    "Paste the URL from your address bar (or a refresh token)...",
                    &state.onboarding.token,
                )
                .on_input(Message::TokenInputChanged)
                .on_submit(Message::TokenSubmitted(state.onboarding.token.clone()))
                .size(14)
                .width(iced::Length::Fill),
                crate::ui::button(iced::widget::text("Continue"))
                    .on_press_maybe(
                        (!state.onboarding.token.is_empty())
                            .then_some(Message::TokenSubmitted(state.onboarding.token.clone())),
                    )
                    .padding(iced::Padding::from([8, 16])),
            ]
            .spacing(8);
            if let Some(error) = error_note {
                instructions = instructions.push(error);
            }

            iced::widget::column![title, instructions]
                .spacing(16)
                .padding(40)
                .width(iced::Length::Fill)
                .into()
        }

        OnboardingStep::ValidatingToken => {
            let spinner = iced::widget::text("Verifying token with Pixiv...").size(14);

            iced::widget::column![title, spinner]
                .spacing(16)
                .padding(40)
                .into()
        }

        OnboardingStep::SelectDirectory => {
            let dir_display = state
                .settings
                .gallery_path
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "Not selected".to_string());

            let dir_section = iced::widget::column![
                iced::widget::text("Select where to store illustrations:").size(14),
                iced::widget::text(dir_display)
                    .size(12)
                    .color(theme::muted(&t)),
                crate::ui::button(iced::widget::text("Choose Directory..."))
                    .on_press(Message::PickDirectory)
                    .padding(iced::Padding::from([8, 16])),
            ]
            .spacing(8);

            let done_btn = crate::ui::button(iced::widget::text("Start Pixiv Manager →"))
                .on_press(Message::OnboardingComplete)
                .padding(iced::Padding::from([10, 24]));

            iced::widget::column![title, dir_section, done_btn]
                .spacing(16)
                .padding(40)
                .into()
        }

        OnboardingStep::Complete => {
            let done = iced::widget::text("Setup complete! Loading...").size(14);
            iced::widget::column![title, done]
                .spacing(16)
                .padding(40)
                .into()
        }
    }
}

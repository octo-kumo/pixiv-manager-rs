//! The download-manager panel — a modal listing the current download queue
//! with per-item status and priority controls (raise/lower order). Earlier in
//! the queue = downloaded sooner.

use crate::app::message::Message;
use crate::app::state::AppState;
use crate::theme;

/// A modal download manager: every queued/downloading/failed item with
/// priority reordering.
pub fn download_manager(state: &AppState) -> iced::Element<'_, Message> {
    let queue = &state.jobs.download_queue;
    let tracker = &state.jobs.downloads;
    let in_flight = &state.jobs.download_job.in_flight;

    let mut rows: Vec<iced::Element<'_, Message>> = Vec::new();

    // ── Queue (queued + downloading), in download order ──
    if queue.remaining.is_empty() && queue.failed.is_empty() {
        rows.push(
            iced::widget::container(iced::widget::text("Queue is empty").size(12))
                .padding(16)
                .into(),
        );
    } else {
        rows.push(
            iced::widget::text(format!(
                "Queue · {} item(s) · {} done",
                queue.remaining.len(),
                queue.done()
            ))
            .size(12)
            .into(),
        );
        for (i, id) in queue.remaining.iter().enumerate() {
            let downloading = in_flight.contains(id);
            let status: iced::Element<'_, Message> = if downloading {
                let frac = tracker
                    .get(*id)
                    .map(|s| s.progress_fraction())
                    .unwrap_or(0.0);
                iced::widget::text(format!("⬇ {:.0}%", frac * 100.0))
                    .size(11)
                    .into()
            } else {
                iced::widget::text("⏳ queued").size(11).into()
            };
            let priority_row = iced::widget::row![
                crate::ui::button(iced::widget::text("↑").size(12))
                    .on_press_maybe((i > 0).then_some(Message::DownloadPriorityUp(*id)))
                    .padding(iced::Padding::from([1, 6])),
                crate::ui::button(iced::widget::text("↓").size(12))
                    .on_press_maybe(
                        (i + 1 < queue.remaining.len())
                            .then_some(Message::DownloadPriorityDown(*id)),
                    )
                    .padding(iced::Padding::from([1, 6])),
            ]
            .spacing(2);

            rows.push(
                iced::widget::container(
                    iced::widget::row![
                        iced::widget::text(format!("#{}", crate::media::native_id(*id)))
                            .size(11)
                            .width(90),
                        status,
                        iced::widget::Space::new().width(iced::Length::Fill),
                        priority_row,
                    ]
                    .spacing(6)
                    .align_y(iced::Alignment::Center),
                )
                .padding(iced::Padding::from([3, 6]))
                .style(|theme| iced::widget::container::Style {
                    background: Some(iced::Background::Color(
                        theme::panel(theme).scale_alpha(0.6),
                    )),
                    border: iced::Border {
                        radius: crate::ui::RADIUS.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                })
                .into(),
            );
        }

        // ── Permanently-failed (deleted) items ──
        if !queue.failed.is_empty() {
            rows.push(
                iced::widget::container(
                    iced::widget::text(format!("Deleted (skipped): {}", queue.failed.len()))
                        .size(11),
                )
                .padding(8)
                .into(),
            );
            for id in queue.failed.iter().take(20) {
                rows.push(
                    iced::widget::row![
                        iced::widget::text(format!("#{}", crate::media::native_id(*id)))
                            .size(10)
                            .width(90),
                        iced::widget::text("✗ deleted")
                            .size(10)
                            .color(crate::theme::DANGER),
                    ]
                    .spacing(6)
                    .padding(iced::Padding::from([2, 6]))
                    .into(),
                );
            }
        }
    }

    let panel = iced::widget::container(
        iced::widget::column![
            iced::widget::row![
                iced::widget::text("Download Manager").size(18),
                iced::widget::Space::new().width(iced::Length::Fill),
                crate::ui::button(iced::widget::text("✕").size(14))
                    .on_press(Message::ToggleDownloadManager)
                    .padding(iced::Padding::from([4, 10]))
                    .style(crate::ui::ghost),
            ]
            .spacing(8),
            crate::ui::scrollable(iced::widget::Column::with_children(rows).spacing(2))
                .height(iced::Length::Fill),
        ]
        .spacing(10)
        .padding(16)
        .width(520)
        .height(480),
    )
    .style(crate::ui::card);

    iced::widget::container(panel)
        .center_x(iced::Length::Fill)
        .center_y(iced::Length::Fill)
        .width(iced::Length::Fill)
        .height(iced::Length::Fill)
        .style(crate::ui::scrim)
        .into()
}

use ratatui::layout::Alignment;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::Frame;

use crate::ui::common::render_dialog_box;
use crate::ui::theme::Theme;

/// Modal for shelving an alarm with custom duration.
pub fn draw(f: &mut Frame, alarm_id: &str, duration_buffer: &str) {
    let text = vec![
        Line::from(vec![
            Span::styled("Alarm: ", Style::default().fg(Theme::SECONDARY)),
            Span::styled(alarm_id, Theme::bold_white()),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "Enter shelve duration in seconds (e.g. 300 for 5m):",
            Style::default().fg(Theme::PRIMARY),
        )),
        Line::from(vec![
            Span::styled("> ", Style::default().fg(Theme::SECONDARY)),
            Span::styled(
                duration_buffer,
                Style::default()
                    .fg(Theme::TEXT)
                    .add_modifier(ratatui::style::Modifier::BOLD)
                    .bg(Theme::BG_INPUT_FIELD),
            ),
            Span::styled("█", Style::default().fg(Theme::PRIMARY)),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "[Enter] Shelve Alarm   [Esc] Cancel",
            Theme::muted(),
        )),
    ];

    render_dialog_box(
        f,
        50,
        25,
        "Shelve Alarm Duration Prompt",
        Style::default().fg(Theme::SECONDARY),
        text,
        Alignment::Center,
    );
}

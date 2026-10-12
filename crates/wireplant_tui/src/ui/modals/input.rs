use ratatui::layout::Alignment;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::Frame;

use crate::ui::common::render_dialog_box;
use crate::ui::theme::Theme;

/// Centered floating modal popup for overriding a tag value.
pub fn draw(
    f: &mut Frame,
    tag_id: &str,
    input_buffer: &str,
    duration_buffer: &str,
    is_discrete: bool,
) {
    let prompt = if is_discrete {
        "Enter 1 for ON, 0 for OFF:"
    } else {
        "Enter target analog value (float/integer):"
    };

    let dur_display = if duration_buffer.is_empty() {
        "Permanent (hold until cleared)".to_string()
    } else {
        format!("{duration_buffer} seconds")
    };

    let text = vec![
        Line::from(vec![
            Span::styled("Target Tag: ", Style::default().fg(Theme::SECONDARY)),
            Span::styled(tag_id, Theme::bold_white()),
        ]),
        Line::from(""),
        Line::from(Span::styled(prompt, Style::default().fg(Theme::PRIMARY))),
        Line::from(vec![
            Span::styled("> ", Theme::bold_magenta()),
            Span::styled(
                input_buffer,
                Style::default()
                    .fg(Theme::TEXT)
                    .add_modifier(ratatui::style::Modifier::BOLD)
                    .bg(Theme::BG_INPUT_FIELD),
            ),
            Span::styled("█", Style::default().fg(Theme::PRIMARY)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("Timed Hold (Press [Tab] to toggle 10s): ", Theme::muted()),
            Span::styled(dur_display, Style::default().fg(Theme::SECONDARY)),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "[Enter] Commit Override   [Esc] Cancel Modal",
            Theme::muted(),
        )),
    ];

    render_dialog_box(
        f,
        55,
        30,
        "Manual Override Value Prompt",
        Theme::bold_magenta(),
        text,
        Alignment::Center,
    );
}

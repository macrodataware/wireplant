use ratatui::layout::Alignment;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::Frame;

use crate::ui::common::render_dialog_box;
use crate::ui::theme::Theme;

/// Modal for searching tags by substring.
pub fn draw(f: &mut Frame, query_buffer: &str) {
    let text = vec![
        Line::from(Span::styled(
            "Filter tags by identifier or description substring:",
            Style::default().fg(Theme::PRIMARY),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("/ ", Style::default().fg(Theme::SECONDARY)),
            Span::styled(
                query_buffer,
                Style::default()
                    .fg(Theme::TEXT)
                    .add_modifier(ratatui::style::Modifier::BOLD)
                    .bg(Theme::BG_INPUT_FIELD),
            ),
            Span::styled("█", Style::default().fg(Theme::PRIMARY)),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "[Enter] Apply Filter   [Esc] Cancel",
            Theme::muted(),
        )),
    ];

    render_dialog_box(
        f,
        50,
        20,
        "Tag Search / Filter",
        Style::default().fg(Theme::PRIMARY),
        text,
        Alignment::Center,
    );
}

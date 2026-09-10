use ratatui::layout::Alignment;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::ui::common::centered_rect;
use crate::ui::theme::Theme;

/// Modal for searching tags by substring.
pub fn draw(f: &mut Frame, query_buffer: &str) {
    let area = centered_rect(50, 20, f.area());
    f.render_widget(Clear, area);

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

    let modal = Paragraph::new(text).alignment(Alignment::Center).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Tag Search / Filter ")
            .border_style(Style::default().fg(Theme::PRIMARY)),
    );

    f.render_widget(modal, area);
}

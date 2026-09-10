use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Row, Table};
use ratatui::Frame;

use crate::app::TuiApp;
use crate::ui::theme::Theme;

/// Tab 4: Network Endpoints and Sessions Dashboard.
pub fn draw(f: &mut Frame, app: &TuiApp, area: Rect) {
    let services = app.get_network_diagnostics();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(12), Constraint::Length(6)])
        .split(area);

    let rows: Vec<Row> = services
        .iter()
        .map(|s| {
            let status_style = if s.is_enabled {
                Theme::bold_green()
            } else {
                Theme::muted()
            };

            Row::new(vec![
                Span::styled(&s.name, Theme::bold_white()),
                Span::raw(&s.endpoint),
                Span::styled(&s.status_text, status_style),
                Span::raw(&s.details),
            ])
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(25),
            Constraint::Length(24),
            Constraint::Length(20),
            Constraint::Min(35),
        ],
    )
    .header(
        Row::new(vec![
            "Service / Protocol",
            "Bind / Broker Address",
            "Status Badge",
            "Connection Details & Metrics",
        ])
        .style(Theme::table_header()),
    )
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Industrial Protocol Endpoints & Server Sessions ")
            .border_style(Style::default().fg(Theme::PRIMARY)),
    );
    f.render_widget(table, chunks[0]);

    // Live metrics card
    let info_text = vec![
        Line::from(vec![
            Span::styled(
                "Active Modbus TCP Sessions: ",
                Style::default().fg(Theme::SECONDARY),
            ),
            Span::styled(format!("{}", app.sessions.active()), Theme::bold_white()),
            Span::raw(" | "),
            Span::styled("Max Connections Allowed: ", Theme::muted()),
            Span::raw(format!("{}", app.scenario.network.modbus.max_connections)),
        ]),
        Line::from(vec![
            Span::styled("Historian Engine: ", Style::default().fg(Theme::PRIMARY)),
            Span::raw(if app.historian.is_some() {
                format!(
                    "Active SQLite WAL logging at '{}'",
                    app.scenario.historian.db_path
                )
            } else {
                "Historian persistence disabled (Volatile in-memory mode)".to_string()
            }),
        ]),
    ];

    let bottom_card = Paragraph::new(info_text).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Infrastructure Health & Session Metrics ")
            .border_style(Theme::muted()),
    );
    f.render_widget(bottom_card, chunks[1]);
}

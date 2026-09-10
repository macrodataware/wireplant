use ratatui::layout::{Constraint, Rect};
use ratatui::style::Style;
use ratatui::widgets::{Block, Borders, Row, Table};
use ratatui::Frame;

use crate::app::{LogFilterLevel, LogLevel, TuiApp, LOG_CAPACITY};
use crate::ui::theme::Theme;

/// Tab 7: Scrollable Event Log.
pub fn draw(f: &mut Frame, app: &mut TuiApp, area: Rect) {
    let filter = app.log_filter;
    let filtered_entries: Vec<&crate::app::LogEntry> = app
        .log_entries
        .iter()
        .filter(|entry| match filter {
            LogFilterLevel::All => true,
            LogFilterLevel::WarnAndError => entry.level != LogLevel::Info,
            LogFilterLevel::ErrorOnly => entry.level == LogLevel::Error,
        })
        .collect();

    let rows: Vec<Row> = filtered_entries
        .iter()
        .rev()
        .map(|entry| {
            let level_style = match entry.level {
                LogLevel::Info => Style::default().fg(Theme::ACCENT_GREEN),
                LogLevel::Warn => Theme::bold_yellow(),
                LogLevel::Error => Theme::bold_red(),
            };

            Row::new(vec![
                format!("+{}s", entry.timestamp_secs),
                entry.level.label().to_string(),
                entry.message.clone(),
            ])
            .style(level_style)
        })
        .collect();

    let auto_scroll_tag = if app.log_auto_scroll {
        " [AUTO-SCROLL: ON] "
    } else {
        " [AUTO-SCROLL: OFF] "
    };

    let log_table = Table::new(
        rows,
        [
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Min(40),
        ],
    )
    .header(
        Row::new(vec!["Timestamp", "Level", "System Event Description"])
            .style(Theme::table_header()),
    )
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!(
                " System Event Log (Filter: {} | Buffer: {}/{}){} ",
                app.log_filter.label(),
                app.log_entries.len(),
                LOG_CAPACITY,
                auto_scroll_tag
            ))
            .border_style(Style::default().fg(Theme::PRIMARY)),
    );

    f.render_stateful_widget(log_table, area, &mut app.log_table_state);
}

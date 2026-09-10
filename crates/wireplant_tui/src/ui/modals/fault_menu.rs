use ratatui::layout::Constraint;
use ratatui::style::Style;
use ratatui::widgets::{Block, Borders, Clear, Row, Table};
use ratatui::Frame;

use crate::app::TuiApp;
use crate::ui::common::centered_rect;
use crate::ui::theme::Theme;

/// Floating modal menu for picking and triggering scenario fault profiles.
pub fn draw(f: &mut Frame, app: &TuiApp, selected_index: usize) {
    let area = centered_rect(65, 45, f.area());
    f.render_widget(Clear, area);

    let rows: Vec<Row> = if app.scenario.fault_profiles.is_empty() {
        vec![Row::new(vec![
            "No predefined fault profiles found in scenario".to_string()
        ])
        .style(Theme::muted())]
    } else {
        app.scenario
            .fault_profiles
            .iter()
            .enumerate()
            .map(|(idx, profile)| {
                let style = if idx == selected_index {
                    Style::default()
                        .fg(Theme::TEXT_INVERTED)
                        .bg(Theme::SECONDARY)
                        .add_modifier(ratatui::style::Modifier::BOLD)
                } else {
                    Style::default().fg(Theme::TEXT)
                };
                let actions_desc = format!("{} actions", profile.actions.len());
                Row::new(vec![
                    profile.name.clone(),
                    actions_desc,
                    profile.description.clone().unwrap_or_default(),
                ])
                .style(style)
            })
            .collect()
    };

    let table = Table::new(
        rows,
        [
            Constraint::Length(26),
            Constraint::Length(12),
            Constraint::Min(20),
        ],
    )
    .header(Row::new(vec!["Profile", "Impact", "Description"]).style(Theme::table_header()))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Select Predefined Fault Profile to Inject ([Enter] Trigger, [Esc] Close) ")
            .border_style(Theme::bold_yellow()),
    );

    f.render_widget(table, area);
}

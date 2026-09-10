use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::widgets::{Block, Borders, Row, Table};
use ratatui::Frame;

use crate::app::TuiApp;
use crate::ui::theme::Theme;

/// Tab 6: Fault Injection Studio.
pub fn draw(f: &mut Frame, app: &TuiApp, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    // Left: Scenario Fault Profiles
    let profile_rows: Vec<Row> = app
        .scenario
        .fault_profiles
        .iter()
        .enumerate()
        .map(|(idx, p)| {
            let desc = p.description.clone().unwrap_or_default();
            Row::new(vec![
                format!("{}: {}", idx + 1, p.name),
                format!("{} actions", p.actions.len()),
                desc,
            ])
            .style(Style::default().fg(Theme::TEXT))
        })
        .collect();

    let profiles_table = Table::new(
        profile_rows,
        [
            Constraint::Length(32),
            Constraint::Length(12),
            Constraint::Length(30),
        ],
    )
    .header(Row::new(vec!["Profile Name", "Actions", "Description"]).style(Theme::table_header()))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Scenario Predefined Fault Profiles (Press [P] to Launch) ")
            .border_style(Style::default().fg(Theme::PRIMARY)),
    );
    f.render_widget(profiles_table, chunks[0]);

    // Right: Currently Active Overrides List
    let now = app.start_time.elapsed().as_secs_f64();
    let override_rows: Vec<Row> = if app.active_overrides.is_empty() {
        vec![Row::new(vec![
            "No active tag overrides".to_string(),
            "—".to_string(),
            "Nominal".to_string(),
        ])
        .style(Theme::muted())]
    } else {
        let mut sorted_overrides: Vec<_> = app.active_overrides.iter().collect();
        sorted_overrides.sort_by_key(|(tag_id, _)| *tag_id);

        sorted_overrides
            .into_iter()
            .map(|(tag_id, active)| {
                let unit = app
                    .scenario
                    .tag(tag_id)
                    .map(|t| t.units.as_str())
                    .unwrap_or("");
                let expiry = match active.expires_at_secs {
                    Some(at) => {
                        let rem = (at - now).max(0.0);
                        format!("{rem:.0}s remaining")
                    }
                    None => "Permanent hold".to_string(),
                };
                Row::new(vec![
                    tag_id.clone(),
                    format!("{:.2} {}", active.value, unit),
                    expiry,
                ])
                .style(Theme::bold_magenta())
            })
            .collect()
    };

    let overrides_table = Table::new(
        override_rows,
        [
            Constraint::Length(25),
            Constraint::Length(18),
            Constraint::Length(16),
        ],
    )
    .header(
        Row::new(vec!["Overridden Tag", "Forced Value", "Hold Status"])
            .style(Theme::table_header()),
    )
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Active Overrides & Chaos Injections (Press [C] to Clear All) ")
            .border_style(Style::default().fg(Theme::ACCENT_MAGENTA)),
    );
    f.render_widget(overrides_table, chunks[1]);
}

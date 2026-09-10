use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Row, Sparkline, Table};
use ratatui::Frame;
use wireplant_engine::alarm::IsaAlarmState;

use crate::app::{TagAreaFilter, TuiApp};
use crate::ui::common::{format_reading, type_label, ABSENT};
use crate::ui::theme::Theme;

/// Tab 1: Process Tags register table and telemetry trend graph.
pub fn draw(f: &mut Frame, app: &mut TuiApp, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(area);

    // Left: Register Table with filtering
    let filtered_tags = app.filtered_tags();
    let rows: Vec<Row> = filtered_tags
        .iter()
        .enumerate()
        .map(|(idx, t)| {
            let val_opt = app
                .active_overrides
                .get(&t.id)
                .map(|o| o.value)
                .or_else(|| {
                    app.sparkline_data
                        .get(&t.id)
                        .and_then(|d| d.back())
                        .copied()
                });

            let is_overridden = app.active_overrides.contains_key(&t.id);
            let val_str = format_reading(val_opt, t.area.is_discrete(), &t.units);
            let override_str = if is_overridden { "FORCED" } else { "AUTO" };

            // Check if this tag has an active alarm
            let alarm_str = app
                .cached_alarms
                .iter()
                .find(|a| a.config.source_tag == t.id)
                .map(|a| match a.state {
                    IsaAlarmState::Normal => "OK",
                    IsaAlarmState::Unacknowledged => "UNACK",
                    IsaAlarmState::Acknowledged => "ACK",
                    IsaAlarmState::ReturnToNormalUnack => "RTN",
                    IsaAlarmState::Latched => "LATCH",
                    IsaAlarmState::Shelved => "SHELVE",
                })
                .unwrap_or("—");

            let mut style = if idx == app.selected_tag_index {
                Style::default().fg(Theme::TEXT_INVERTED).bg(Theme::PRIMARY)
            } else if is_overridden {
                Theme::bold_magenta()
            } else {
                Style::default().fg(Theme::TEXT)
            };

            if idx == app.selected_tag_index && is_overridden {
                style = Style::default()
                    .fg(Theme::TEXT_INVERTED)
                    .bg(Theme::ACCENT_MAGENTA);
            }

            Row::new(vec![
                t.id.clone(),
                t.area.display_name().to_string(),
                t.address.to_string(),
                val_str,
                t.units.clone(),
                override_str.to_string(),
                alarm_str.to_string(),
            ])
            .style(style)
        })
        .collect();

    let filter_info =
        if !app.tag_search_query.is_empty() || app.tag_area_filter != TagAreaFilter::All {
            format!(
                " Filter: '{}' | Area: {} | (Showing {} of {}) ",
                app.tag_search_query,
                app.tag_area_filter.label(),
                filtered_tags.len(),
                app.scenario.tags.len()
            )
        } else {
            format!(" Process Image Registers ({}) ", app.scenario.tags.len())
        };

    let table = Table::new(
        rows,
        [
            Constraint::Length(22),
            Constraint::Length(16),
            Constraint::Length(5),
            Constraint::Length(10),
            Constraint::Length(7),
            Constraint::Length(7),
            Constraint::Length(8),
        ],
    )
    .header(
        Row::new(vec![
            "Tag ID", "Area", "Addr", "Value", "Units", "Mode", "Alarm",
        ])
        .style(Theme::table_header()),
    )
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(filter_info)
            .border_style(Style::default().fg(Theme::PRIMARY)),
    );
    f.render_stateful_widget(table, chunks[0], &mut app.tag_table_state);

    // Right: Live Telemetry Trend Graph & Summary Stats
    let selected_tag = app.selected_tag().cloned();
    if let Some(tag) = selected_tag {
        let right_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(8), Constraint::Length(4)])
            .split(chunks[1]);

        let spark_vals: Vec<u64> = app
            .sparkline_data
            .get(&tag.id)
            .map(|data| {
                data.iter()
                    .map(|&v| {
                        if v.is_nan() || v < 0.0 {
                            0
                        } else {
                            v.round() as u64
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();

        let raw_f64s: Vec<f64> = app
            .sparkline_data
            .get(&tag.id)
            .map(|data| data.iter().copied().collect())
            .unwrap_or_default();

        let min_val = raw_f64s.iter().fold(f64::INFINITY, |a, &b| a.min(b));
        let max_val = raw_f64s.iter().fold(f64::NEG_INFINITY, |a, &b| a.max(b));
        let avg_val = if raw_f64s.is_empty() {
            None
        } else {
            Some(raw_f64s.iter().sum::<f64>() / (raw_f64s.len() as f64))
        };
        let current_val = raw_f64s.last().copied();

        let title = if tag.units.is_empty() {
            format!(" Live Telemetry: {} ", tag.id)
        } else {
            format!(" Live Telemetry: {} ({}) ", tag.id, tag.units)
        };

        let sparkline = Sparkline::default()
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(title)
                    .border_style(Style::default().fg(Theme::ACCENT_GREEN)),
            )
            .style(Style::default().fg(Theme::ACCENT_GREEN))
            .data(&spark_vals);
        f.render_widget(sparkline, right_chunks[0]);

        let (min_str, max_str) = if min_val.is_infinite() {
            (ABSENT.to_string(), ABSENT.to_string())
        } else {
            (format!("{min_val:.2}"), format!("{max_val:.2}"))
        };

        let current_str = match current_val {
            Some(value) => format!("{value:.2} {}", tag.units),
            None => ABSENT.to_string(),
        };
        let avg_str = match avg_val {
            Some(value) => format!("{value:.2}"),
            None => ABSENT.to_string(),
        };

        let stats_text = vec![
            Line::from(vec![
                Span::styled("Current: ", Style::default().fg(Theme::SECONDARY)),
                Span::styled(format!("{current_str}  "), Theme::bold_white()),
                Span::styled("Min: ", Theme::muted()),
                Span::styled(format!("{min_str}  "), Style::default().fg(Theme::TEXT)),
                Span::styled("Max: ", Theme::muted()),
                Span::styled(format!("{max_str}  "), Style::default().fg(Theme::TEXT)),
                Span::styled("Avg: ", Theme::muted()),
                Span::styled(avg_str, Style::default().fg(Theme::TEXT)),
            ]),
            Line::from(vec![Span::styled(
                format!(
                    "Endianness: {} | Data Type: {}",
                    tag.endianness.wire_order(),
                    type_label(tag.data_type)
                ),
                Theme::muted(),
            )]),
        ];

        let stats_widget = Paragraph::new(stats_text).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Signal Statistics "),
        );
        f.render_widget(stats_widget, right_chunks[1]);
    } else {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Live Telemetry ")
            .border_style(Theme::muted());
        f.render_widget(block, chunks[1]);
    }
}

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Row, Table};
use ratatui::Frame;
use wireplant_engine::alarm::{AlarmSeverity, IsaAlarmState};

use crate::app::TuiApp;
use crate::ui::theme::Theme;

/// Tab 2: ANSI/ISA-18.2 Alarm Annunciator & Management Console.
pub fn draw(f: &mut Frame, app: &mut TuiApp, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(area);

    // Left: Annunciator Table
    let rows: Vec<Row> = app
        .cached_alarms
        .iter()
        .enumerate()
        .map(|(idx, a)| {
            let sev_style = match a.config.severity {
                AlarmSeverity::Critical => Theme::bold_red(),
                AlarmSeverity::High => Style::default().fg(Theme::DANGER_LIGHT),
                AlarmSeverity::Medium => Style::default().fg(Theme::SECONDARY),
                AlarmSeverity::Low => Style::default().fg(Theme::PRIMARY),
            };

            let state_str = match a.state {
                IsaAlarmState::Normal => "NORMAL",
                IsaAlarmState::Unacknowledged => "UNACK",
                IsaAlarmState::Acknowledged => "ACK",
                IsaAlarmState::ReturnToNormalUnack => "RTN_UNACK",
                IsaAlarmState::Latched => "LATCHED",
                IsaAlarmState::Shelved => "SHELVED",
            };

            let state_style = match a.state {
                IsaAlarmState::Unacknowledged | IsaAlarmState::ReturnToNormalUnack => {
                    Style::default()
                        .fg(Theme::TEXT)
                        .bg(Theme::DANGER)
                        .add_modifier(Modifier::BOLD)
                }
                IsaAlarmState::Acknowledged => Theme::bold_yellow(),
                IsaAlarmState::Latched => Theme::bold_magenta(),
                IsaAlarmState::Shelved => Theme::muted(),
                IsaAlarmState::Normal => Style::default().fg(Theme::ACCENT_GREEN),
            };

            let selected = app.alarm_table_state.selected().unwrap_or(0);
            let row_style = if idx == selected {
                Theme::selected_row()
            } else {
                Style::default()
            };

            Row::new(vec![
                Span::styled(a.config.severity.label(), sev_style),
                Span::raw(a.config.id.clone()),
                Span::raw(a.config.source_tag.clone()),
                Span::raw(format!("{:.2} / {:.2}", a.current_value, a.config.limit)),
                Span::styled(state_str, state_style),
            ])
            .style(row_style)
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(10),
            Constraint::Length(18),
            Constraint::Length(18),
            Constraint::Length(16),
            Constraint::Length(12),
        ],
    )
    .header(
        Row::new(vec![
            "Severity",
            "Alarm ID",
            "Source Tag",
            "PV / Limit",
            "State",
        ])
        .style(Theme::table_header()),
    )
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!(
                " ANSI/ISA-18.2 Alarm Annunciator (Total: {}) ",
                app.cached_alarms.len()
            ))
            .border_style(Style::default().fg(Theme::PRIMARY)),
    );
    f.render_stateful_widget(table, chunks[0], &mut app.alarm_table_state);

    // Right: Selected Alarm Inspector & State Machine Details
    let selected_alarm = app.selected_alarm().cloned();
    if let Some(alarm) = selected_alarm {
        let deadband_desc = match alarm.config.alarm_type {
            wireplant_engine::alarm::AlarmType::High
            | wireplant_engine::alarm::AlarmType::HighHigh => {
                format!(
                    "{:.2} (Clears when PV < {:.2})",
                    alarm.config.deadband,
                    alarm.config.limit - alarm.config.deadband
                )
            }
            wireplant_engine::alarm::AlarmType::Low
            | wireplant_engine::alarm::AlarmType::LowLow => {
                format!(
                    "{:.2} (Clears when PV > {:.2})",
                    alarm.config.deadband,
                    alarm.config.limit + alarm.config.deadband
                )
            }
        };

        let shelve_info = if alarm.state == IsaAlarmState::Shelved {
            format!("Remaining: {:.0}s", alarm.shelved_remaining_secs())
        } else {
            format!("Max: {:.0}s", alarm.config.max_shelve_duration_secs)
        };

        let detail_lines = vec![
            Line::from(vec![
                Span::styled("Alarm ID: ", Style::default().fg(Theme::SECONDARY)),
                Span::styled(&alarm.config.id, Theme::bold_white()),
                Span::raw(" | "),
                Span::styled("Severity: ", Theme::muted()),
                Span::styled(
                    alarm.config.severity.label(),
                    Style::default().fg(Theme::DANGER_LIGHT),
                ),
            ]),
            Line::from(vec![
                Span::styled("Description: ", Theme::muted()),
                Span::raw(&alarm.config.description),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("Source Tag: ", Style::default().fg(Theme::PRIMARY)),
                Span::styled(&alarm.config.source_tag, Style::default().fg(Theme::TEXT)),
                Span::raw(" | "),
                Span::styled("Trip Limit: ", Theme::muted()),
                Span::styled(
                    format!("{:.2} ({:?})", alarm.config.limit, alarm.config.alarm_type),
                    Style::default().fg(Theme::SECONDARY),
                ),
            ]),
            Line::from(vec![
                Span::styled("Current Value: ", Theme::muted()),
                Span::styled(format!("{:.2}", alarm.current_value), Theme::bold_white()),
                Span::raw(" | "),
                Span::styled("Condition Active: ", Theme::muted()),
                Span::styled(
                    if alarm.condition_active {
                        "TRUE (IN ALARM)"
                    } else {
                        "FALSE (NORMAL)"
                    },
                    if alarm.condition_active {
                        Theme::bold_red()
                    } else {
                        Style::default().fg(Theme::ACCENT_GREEN)
                    },
                ),
            ]),
            Line::from(vec![
                Span::styled("Hysteresis Deadband: ", Theme::muted()),
                Span::raw(deadband_desc),
            ]),
            Line::from(vec![
                Span::styled("Delays: On=", Theme::muted()),
                Span::raw(format!("{:.1}s", alarm.config.on_delay_secs)),
                Span::styled(", Off=", Theme::muted()),
                Span::raw(format!("{:.1}s", alarm.config.off_delay_secs)),
                Span::raw(" | "),
                Span::styled("Latching: ", Theme::muted()),
                Span::raw(if alarm.config.latching { "YES" } else { "NO" }),
            ]),
            Line::from(vec![
                Span::styled("Shelve Status: ", Theme::muted()),
                Span::raw(shelve_info),
            ]),
            Line::from(""),
            Line::from(Span::styled(
                "SCADA Process Image Mappings:",
                Style::default().fg(Theme::SECONDARY),
            )),
            Line::from(vec![
                Span::styled("Active Coil: ", Theme::muted()),
                Span::raw(
                    alarm
                        .config
                        .active_coil_addr
                        .map(|a| format!("C:{a}"))
                        .unwrap_or("None".into()),
                ),
                Span::styled(" | Unack Coil: ", Theme::muted()),
                Span::raw(
                    alarm
                        .config
                        .unack_coil_addr
                        .map(|a| format!("C:{a}"))
                        .unwrap_or("None".into()),
                ),
                Span::styled(" | State Reg: ", Theme::muted()),
                Span::raw(
                    alarm
                        .config
                        .state_reg_addr
                        .map(|a| format!("HR:{a}"))
                        .unwrap_or("None".into()),
                ),
            ]),
            Line::from(""),
            Line::from(Span::styled(
                "Operator Actions: [a] Acknowledge  [r] Reset  [s] Shelve  [u] Unshelve",
                Style::default().fg(Theme::ACCENT_MAGENTA),
            )),
        ];

        let panel = Paragraph::new(detail_lines).block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" Alarm Inspector: {} ", alarm.config.id))
                .border_style(Style::default().fg(Theme::PRIMARY)),
        );
        f.render_widget(panel, chunks[1]);
    } else {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Alarm Inspector (No Alarm Selected) ")
            .border_style(Theme::muted());
        f.render_widget(block, chunks[1]);
    }
}

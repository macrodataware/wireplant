use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::app::TuiApp;
use crate::ui::theme::Theme;

/// Tab 3: Physical Process Models (Tank, Thermal) & PID Controller Loops.
pub fn draw(f: &mut Frame, app: &TuiApp, area: Rect) {
    let (tanks, thermals, pids) = app.get_process_states();

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    // Left Column: Physical Process Units
    let left_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(chunks[0]);

    // 1. Tanks
    if tanks.is_empty() {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Hydraulic Tank Units (None in Scenario) ")
            .border_style(Theme::muted());
        f.render_widget(block, left_chunks[0]);
    } else {
        let mut tank_lines = Vec::new();
        for t in &tanks {
            let ratio = (t.current_level / t.max_height).clamp(0.0, 1.0);
            tank_lines.push(Line::from(vec![
                Span::styled(
                    format!("Tank: Level Tag '{}'", t.level_tag),
                    Theme::bold_cyan(),
                ),
                Span::raw(format!(" (Surface Area: {:.1} m²)", t.surface_area)),
            ]));
            tank_lines.push(Line::from(vec![
                Span::styled("Level: ", Style::default().fg(Theme::SECONDARY)),
                Span::styled(
                    format!(
                        "{:.2}m / {:.2}m ({:.1}%)",
                        t.current_level,
                        t.max_height,
                        ratio * 100.0
                    ),
                    Theme::bold_white(),
                ),
            ]));
            tank_lines.push(Line::from(vec![
                Span::styled("Inflow Rate: ", Theme::muted()),
                Span::raw(format!("{:.2} m³/s", t.inflow_rate)),
                Span::styled(" | Outflow Rate: ", Theme::muted()),
                Span::raw(format!("{:.2} m³/s", t.outflow_rate)),
            ]));
            tank_lines.push(Line::from(vec![
                Span::styled("Gravity Drain Coeff: ", Theme::muted()),
                Span::raw(format!("{:.3}", t.gravity_drain_coeff)),
                Span::styled(" | Net Rate: ", Theme::muted()),
                Span::raw(format!("{:+.2} m³/s", t.inflow_rate - t.outflow_rate)),
            ]));
            tank_lines.push(Line::from(""));
        }

        let tank_panel = Paragraph::new(tank_lines).block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" Hydraulic Tanks ({}) ", tanks.len()))
                .border_style(Style::default().fg(Theme::PRIMARY)),
        );
        f.render_widget(tank_panel, left_chunks[0]);
    }

    // 2. Thermals
    if thermals.is_empty() {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Thermal Process Units (None in Scenario) ")
            .border_style(Theme::muted());
        f.render_widget(block, left_chunks[1]);
    } else {
        let mut thermal_lines = Vec::new();
        for th in &thermals {
            thermal_lines.push(Line::from(vec![Span::styled(
                format!("Thermal Unit: Tag '{}'", th.temp_tag),
                Style::default()
                    .fg(Theme::DANGER_LIGHT)
                    .add_modifier(ratatui::style::Modifier::BOLD),
            )]));
            thermal_lines.push(Line::from(vec![
                Span::styled("Current Temp: ", Style::default().fg(Theme::SECONDARY)),
                Span::styled(format!("{:.2} °C", th.current_temp), Theme::bold_white()),
                Span::styled(" | Ambient: ", Theme::muted()),
                Span::raw(format!("{:.1} °C", th.ambient_temp)),
            ]));
            thermal_lines.push(Line::from(vec![
                Span::styled("Heater Input: ", Theme::muted()),
                Span::raw(format!("{:.1} %", th.heater_input * 100.0)),
            ]));
            thermal_lines.push(Line::from(""));
        }

        let thermal_panel = Paragraph::new(thermal_lines).block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" Thermal Heat Exchangers ({}) ", thermals.len()))
                .border_style(Style::default().fg(Theme::DANGER_LIGHT)),
        );
        f.render_widget(thermal_panel, left_chunks[1]);
    }

    // Right Column: Closed-Loop PID Controllers
    if pids.is_empty() {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Closed-Loop PID Controllers (None in Scenario) ")
            .border_style(Theme::muted());
        f.render_widget(block, chunks[1]);
    } else {
        let mut pid_lines = Vec::new();
        for (i, pid) in pids.iter().enumerate() {
            pid_lines.push(Line::from(vec![Span::styled(
                format!("Loop #{}: Output Tag '{}'", i + 1, pid.output_tag),
                Theme::bold_yellow(),
            )]));
            pid_lines.push(Line::from(vec![
                Span::styled("Setpoint (SP): ", Style::default().fg(Theme::PRIMARY)),
                Span::styled(format!("{:.2}", pid.setpoint), Theme::bold_white()),
                Span::styled(
                    " | Process Var (PV): ",
                    Style::default().fg(Theme::ACCENT_GREEN),
                ),
                Span::styled(format!("{:.2}", pid.process_variable), Theme::bold_white()),
            ]));
            pid_lines.push(Line::from(vec![
                Span::styled("Error (SP - PV): ", Theme::muted()),
                Span::styled(
                    format!("{:+.2}", pid.error),
                    if pid.error.abs() > 1.0 {
                        Style::default().fg(Theme::DANGER)
                    } else {
                        Style::default().fg(Theme::TEXT)
                    },
                ),
                Span::styled(
                    " | Control Output: ",
                    Style::default().fg(Theme::ACCENT_MAGENTA),
                ),
                Span::styled(format!("{:.2}", pid.output), Theme::bold_white()),
            ]));
            pid_lines.push(Line::from(vec![
                Span::styled("Tuning: Kp=", Theme::muted()),
                Span::raw(format!("{:.2}", pid.config.kp)),
                Span::styled(", Ki=", Theme::muted()),
                Span::raw(format!("{:.2}", pid.config.ki)),
                Span::styled(", Kd=", Theme::muted()),
                Span::raw(format!("{:.2}", pid.config.kd)),
                Span::styled(" | Anti-Windup: ", Theme::muted()),
                Span::raw(if pid.config.anti_windup {
                    "ENABLED"
                } else {
                    "DISABLED"
                }),
            ]));
            pid_lines.push(Line::from(Span::styled(
                "Press [s] to override Setpoint",
                Theme::muted(),
            )));
            pid_lines.push(Line::from(""));
        }

        let pid_panel = Paragraph::new(pid_lines).block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" Closed-Loop PID Controllers ({}) ", pids.len()))
                .border_style(Style::default().fg(Theme::SECONDARY)),
        );
        f.render_widget(pid_panel, chunks[1]);
    }
}

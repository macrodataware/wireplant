pub mod common;
pub mod modals;
pub mod tabs;
pub mod theme;

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::{Block, Borders, Paragraph, Tabs};
use ratatui::Frame;

use crate::app::{ActiveTab, TuiApp};
use theme::Theme;

const TAB_TITLES: &[&str] = &[
    " 1: Tags ",
    " 2: Alarms ",
    " 3: Control Loops ",
    " 4: Network & Svc ",
    " 5: Memory Map ",
    " 6: Fault Studio ",
    " 7: Event Log ",
];

/// Top navigation tabs and live facility telemetry badges.
fn draw_top_bar(f: &mut Frame, app: &TuiApp, area: Rect) {
    let top_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(3)])
        .split(area);

    let tabs = Tabs::new(TAB_TITLES.iter().copied())
        .select(app.active_tab.index())
        .style(Theme::muted())
        .highlight_style(
            Style::default()
                .fg(Theme::PRIMARY)
                .add_modifier(Modifier::BOLD)
                .bg(Theme::BG_TAB_SELECTED),
        )
        .divider(Span::raw("|"));
    f.render_widget(tabs, top_chunks[0]);

    let uptime = app.start_time.elapsed().as_secs();
    let overrides_count = app.active_overrides.len();
    let override_badge = if overrides_count > 0 {
        format!(" | FORCED: [{overrides_count}]")
    } else {
        String::new()
    };

    let alarm_badge = if app.unacknowledged_alarms_count > 0 {
        format!(
            " | ALARMS: [{} UNACK / {} ACTIVE]",
            app.unacknowledged_alarms_count, app.active_alarms_count
        )
    } else if app.active_alarms_count > 0 {
        format!(" | ALARMS: [{} ACTIVE]", app.active_alarms_count)
    } else {
        " | ALARMS: [NORMAL]".to_string()
    };

    let status_line = format!(
        " Facility: {} | Uptime: {}s | Rate: {}ms | Tags: {}{}{}",
        app.scenario.facility.name,
        uptime,
        app.scenario.facility.tick_rate_ms,
        app.scenario.tags.len(),
        override_badge,
        alarm_badge,
    );

    let border_style = if app.unacknowledged_alarms_count > 0 {
        Theme::bold_red()
    } else if overrides_count > 0 {
        Style::default().fg(Theme::ACCENT_MAGENTA)
    } else {
        Style::default().fg(Theme::PRIMARY)
    };

    let header_widget = Paragraph::new(status_line)
        .style(Theme::bold_white())
        .block(
            Block::default()
                .borders(Borders::BOTTOM)
                .border_style(border_style),
        );
    f.render_widget(header_widget, top_chunks[1]);
}

/// Contextual bottom footer bar.
fn draw_footer(f: &mut Frame, app: &TuiApp, area: Rect) {
    let specific_hints = match app.active_tab {
        ActiveTab::ProcessTags => {
            "[↑/↓] Select | [/] Search | [m] Area Filter | [Enter/F] Override | [Space] Toggle | [z] Freeze"
        }
        ActiveTab::Alarms => {
            "[↑/↓] Select | [a] Ack | [r] Reset | [s] Shelve | [u] Unshelve"
        }
        ActiveTab::ProcessControl => "[s/Enter] Override Setpoint | [Tab] Cycle",
        ActiveTab::NetworkSessions => "Live session telemetry | [Tab] Cycle",
        ActiveTab::MemoryMap => "[1-4] Table | [↑/↓] Offset | [PgUp/PgDn] Page",
        ActiveTab::FaultStudio => "[P] Trigger Fault | [c] Clear Selected | [C] Clear All",
        ActiveTab::EventLog => "[↑/↓] Scroll | [PgUp/PgDn] Page | [a] Auto-Scroll | [f] Level Filter",
    };

    let text = format!(" [1-7] Tabs | {specific_hints} | [?] Help | [Q] Quit");
    let footer = Paragraph::new(text).style(Theme::muted()).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Controls & Shortcuts "),
    );
    f.render_widget(footer, area);
}

/// Paints one full frame: header, active tab, footer, and any active modal.
pub fn draw_ui(f: &mut Frame, app: &mut TuiApp) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4), // Tabs + facility banner
            Constraint::Min(10),   // Active tab contents
            Constraint::Length(3), // Footer hints
        ])
        .split(f.area());

    draw_top_bar(f, app, chunks[0]);

    match app.active_tab {
        ActiveTab::ProcessTags => tabs::tags::draw(f, app, chunks[1]),
        ActiveTab::Alarms => tabs::alarms::draw(f, app, chunks[1]),
        ActiveTab::ProcessControl => tabs::control::draw(f, app, chunks[1]),
        ActiveTab::NetworkSessions => tabs::network::draw(f, app, chunks[1]),
        ActiveTab::MemoryMap => tabs::memory_map::draw(f, app, chunks[1]),
        ActiveTab::FaultStudio => tabs::fault_studio::draw(f, app, chunks[1]),
        ActiveTab::EventLog => tabs::event_log::draw(f, app, chunks[1]),
    }

    draw_footer(f, app, chunks[2]);
    modals::draw_active_modal(f, app);
}

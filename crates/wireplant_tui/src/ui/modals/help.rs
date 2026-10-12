use ratatui::layout::Alignment;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::Frame;

use crate::ui::common::render_dialog_box;
use crate::ui::theme::Theme;

/// Global Hotkey and Navigation Help Modal.
pub fn draw(f: &mut Frame) {
    let text = vec![
        Line::from(vec![
            Span::styled("Global Navigation: ", Style::default().fg(Theme::PRIMARY)),
            Span::raw("[1-7] Direct Tab Jump  |  [Tab / Shift-Tab] Cycle Tabs  |  [Q / Esc] Quit"),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "Tab 1: Process Tags",
            Style::default().fg(Theme::SECONDARY),
        )),
        Line::from("  [↑/↓] Select Tag   [PgUp/PgDn] Page Scroll   [/] Search Substring"),
        Line::from("  [m] Cycle Memory Area Filter (All -> HoldReg -> InputReg -> Coil -> DI)"),
        Line::from("  [Enter / F] Override Analog/Float Value   [Space] Toggle Digital Bit"),
        Line::from(
            "  [z] Freeze Value at Current Reading   [c] Clear Tag Override   [C] Clear All",
        ),
        Line::from(""),
        Line::from(Span::styled(
            "Tab 2: ANSI/ISA-18.2 Alarms",
            Style::default().fg(Theme::SECONDARY),
        )),
        Line::from("  [a] Acknowledge Alarm (UNACK -> ACK)   [r] Reset Latched Alarm"),
        Line::from("  [s] Shelve Alarm (Prompt Duration)   [u] Unshelve Alarm Immediately"),
        Line::from(""),
        Line::from(Span::styled(
            "Tab 3: Control Loops & Physics",
            Style::default().fg(Theme::SECONDARY),
        )),
        Line::from("  [s] Adjust PID Setpoint   [Enter] Inspect Process Details"),
        Line::from(""),
        Line::from(Span::styled(
            "Tab 5: Memory Map (Raw Process Image)",
            Style::default().fg(Theme::SECONDARY),
        )),
        Line::from(
            "  [1-4] Select Table (Coils / DI / IR / HR)   [↑/↓] Offset Scroll   [PgUp/PgDn] Page",
        ),
        Line::from(""),
        Line::from(Span::styled(
            "Tab 7: System Event Log",
            Style::default().fg(Theme::SECONDARY),
        )),
        Line::from("  [↑/↓] Scroll History   [PgUp/PgDn] Page   [Home/End] Jump Top/Bottom"),
        Line::from(
            "  [a] Toggle Auto-Scroll Follow   [f] Cycle Filter Level (All -> Warn/Err -> Err)",
        ),
        Line::from(""),
        Line::from(Span::styled(
            "Press [Esc] or [?] to close this help dialog",
            Theme::muted(),
        )),
    ];

    render_dialog_box(
        f,
        70,
        70,
        "Keyboard & Operator Command Reference ([Esc] Close)",
        Style::default().fg(Theme::SECONDARY),
        text,
        Alignment::Left,
    );
}

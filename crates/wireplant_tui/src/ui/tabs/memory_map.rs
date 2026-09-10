use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::Span;
use ratatui::widgets::{Block, Borders, Paragraph, Row, Table, Tabs};
use ratatui::Frame;

use crate::app::{MemoryTableSelection, TuiApp};
use crate::ui::theme::Theme;

/// Tab 5: Raw Process Image Memory Inspector.
pub fn draw(f: &mut Frame, app: &mut TuiApp, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(8)])
        .split(area);

    const MEMORY_TABLE_TITLES: &[&str] = &[
        " 1: Coils ",
        " 2: Discrete Inputs ",
        " 3: Input Registers ",
        " 4: Holding Registers ",
    ];
    let sel_idx = match app.memory_table_selected {
        MemoryTableSelection::Coils => 0,
        MemoryTableSelection::DiscreteInputs => 1,
        MemoryTableSelection::InputRegisters => 2,
        MemoryTableSelection::HoldingRegisters => 3,
    };
    let sub_tabs = Tabs::new(MEMORY_TABLE_TITLES.iter().copied())
        .select(sel_idx)
        .style(Theme::muted())
        .highlight_style(
            Style::default()
                .fg(Theme::SECONDARY)
                .add_modifier(ratatui::style::Modifier::BOLD)
                .bg(ratatui::style::Color::Rgb(30, 45, 65)),
        )
        .divider(Span::raw("|"))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Select Memory Table (Press [1-4]) "),
        );
    f.render_widget(sub_tabs, chunks[0]);

    // Grid rendering based on selected table
    let mem_guard = match app.shared_memory.try_read() {
        Ok(guard) => guard,
        Err(_) => {
            let p = Paragraph::new("Process Image memory is currently locked by a simulation tick")
                .style(Style::default().fg(Theme::SECONDARY));
            f.render_widget(p, chunks[1]);
            return;
        }
    };

    let start_offset = app.memory_address_offset;
    let rows: Vec<Row> = match app.memory_table_selected {
        MemoryTableSelection::HoldingRegisters | MemoryTableSelection::InputRegisters => {
            let is_hr = app.memory_table_selected == MemoryTableSelection::HoldingRegisters;
            let target_slice = if is_hr {
                &mem_guard.holding_registers
            } else {
                &mem_guard.input_registers
            };
            let area_type = if is_hr {
                wireplant_core::types::MemoryArea::HoldingRegister
            } else {
                wireplant_core::types::MemoryArea::InputRegister
            };

            target_slice
                .chunks(2)
                .enumerate()
                .skip(start_offset)
                .take(30)
                .map(|(pair_idx, words)| {
                    let addr = pair_idx * 2;
                    let w0 = words.first().copied().unwrap_or(0);
                    let w1 = words.get(1).copied().unwrap_or(0);
                    let hex_str = format!("0x{w0:04X} 0x{w1:04X}");
                    let f32_val = wireplant_core::endian::registers_to_float32(
                        w0,
                        w1,
                        wireplant_core::types::Endianness::BigEndian,
                    );

                    let tag_name = app
                        .scenario
                        .tags
                        .iter()
                        .find(|t| t.area == area_type && t.address as usize == addr)
                        .map(|t| t.id.as_str())
                        .unwrap_or("—");

                    Row::new(vec![
                        Span::raw(format!("0x{addr:04X} ({addr})")),
                        Span::styled(hex_str, Style::default().fg(Theme::PRIMARY)),
                        Span::raw(format!("{f32_val:.2} (f32)")),
                        Span::raw(format!("{w0} (u16)")),
                        Span::styled(tag_name, Style::default().fg(Theme::SECONDARY)),
                    ])
                })
                .collect()
        }
        MemoryTableSelection::Coils | MemoryTableSelection::DiscreteInputs => {
            let is_coil = app.memory_table_selected == MemoryTableSelection::Coils;
            let target_slice = if is_coil {
                &mem_guard.coils
            } else {
                &mem_guard.discrete_inputs
            };
            let area_type = if is_coil {
                wireplant_core::types::MemoryArea::Coil
            } else {
                wireplant_core::types::MemoryArea::DiscreteInput
            };

            target_slice
                .iter()
                .enumerate()
                .skip(start_offset)
                .take(30)
                .map(|(addr, &bit)| {
                    let tag_name = app
                        .scenario
                        .tags
                        .iter()
                        .find(|t| t.area == area_type && t.address as usize == addr)
                        .map(|t| t.id.as_str())
                        .unwrap_or("—");

                    let bit_str = if bit { "1 (TRUE)" } else { "0 (FALSE)" };
                    let bit_style = if bit {
                        Theme::bold_green()
                    } else {
                        Theme::muted()
                    };

                    Row::new(vec![
                        Span::raw(format!("0x{addr:04X} ({addr})")),
                        Span::styled(bit_str, bit_style),
                        Span::raw("—"),
                        Span::raw("—"),
                        Span::styled(tag_name, Style::default().fg(Theme::SECONDARY)),
                    ])
                })
                .collect()
        }
    };

    let title = format!(
        " Raw Process Image: {} (Offset: +{}) ",
        app.memory_table_selected.label(),
        start_offset
    );

    let table = Table::new(
        rows,
        [
            Constraint::Length(18),
            Constraint::Length(22),
            Constraint::Length(18),
            Constraint::Length(14),
            Constraint::Min(20),
        ],
    )
    .header(
        Row::new(vec![
            "Address Offset",
            "Raw Data (Hex / Bit)",
            "Float32 Interpretation",
            "Unsigned Word",
            "Mapped Tag ID",
        ])
        .style(Theme::table_header()),
    )
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(title)
            .border_style(Style::default().fg(Theme::PRIMARY)),
    );

    f.render_stateful_widget(table, chunks[1], &mut app.memory_table_state);
}

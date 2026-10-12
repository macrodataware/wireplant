use ratatui::layout::{Constraint, Direction, Layout, Rect};

/// Shown wherever a reading does not exist yet.
pub const ABSENT: &str = "N/A";

/// Formats a physical reading with unit suffixes or discrete state labels.
pub fn format_reading(value: Option<f64>, is_discrete: bool, units: &str) -> String {
    match value {
        None => ABSENT.to_string(),
        Some(value) if is_discrete => {
            if value >= wireplant_core::codec::BOOL_THRESHOLD {
                "ON (1)".to_string()
            } else {
                "OFF (0)".to_string()
            }
        }
        Some(value) if units.is_empty() => format!("{value:.2}"),
        Some(value) => format!("{value:.2} {units}"),
    }
}

/// Returns true if the memory area addresses binary discrete bits (Coil or DiscreteInput).
#[inline]
pub fn is_discrete_area(area: wireplant_core::types::MemoryArea) -> bool {
    area.is_discrete()
}

/// Appends an item to a VecDeque ring buffer, popping from front when capacity is reached.
pub fn push_capped<T>(buf: &mut std::collections::VecDeque<T>, item: T, capacity: usize) {
    if buf.len() >= capacity {
        buf.pop_front();
    }
    buf.push_back(item);
}

/// Returns a human-readable abbreviation for a tag's data type.
pub fn type_label(data_type: wireplant_core::types::TagDataType) -> &'static str {
    use wireplant_core::types::TagDataType::*;
    match data_type {
        Boolean => "bool",
        Int16 => "int16",
        UInt16 => "uint16",
        Int32 => "int32",
        UInt32 => "uint32",
        Float32 => "float32",
        Float64 => "float64",
    }
}

/// Builds a `Rect` centered on both axes by given percentage dimensions.
pub fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

/// Renders a centered floating modal dialog with an opaque background.
pub fn render_dialog_box(
    f: &mut ratatui::Frame,
    percent_x: u16,
    percent_y: u16,
    title: &str,
    border_style: ratatui::style::Style,
    lines: Vec<ratatui::text::Line>,
    alignment: ratatui::layout::Alignment,
) {
    let area = centered_rect(percent_x, percent_y, f.area());
    f.render_widget(ratatui::widgets::Clear, area);

    let modal = ratatui::widgets::Paragraph::new(lines)
        .alignment(alignment)
        .block(
            ratatui::widgets::Block::default()
                .borders(ratatui::widgets::Borders::ALL)
                .title(format!(" {title} "))
                .border_style(border_style),
        );

    f.render_widget(modal, area);
}

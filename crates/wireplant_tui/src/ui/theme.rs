use ratatui::style::{Color, Modifier, Style};

/// Centralized UI design tokens and style presets for consistent dashboard aesthetics.
#[derive(Debug)]
pub struct Theme;

impl Theme {
    // Primary palette
    pub const PRIMARY: Color = Color::Cyan;
    pub const SECONDARY: Color = Color::Yellow;
    pub const ACCENT_GREEN: Color = Color::Green;
    pub const ACCENT_MAGENTA: Color = Color::Magenta;
    pub const DANGER: Color = Color::Red;
    pub const DANGER_LIGHT: Color = Color::LightRed;
    pub const MUTED: Color = Color::DarkGray;
    pub const TEXT: Color = Color::White;
    pub const TEXT_INVERTED: Color = Color::Black;

    // Background selection tints
    pub const BG_ROW_SELECTED: Color = Color::Rgb(25, 40, 60);
    pub const BG_TAB_SELECTED: Color = Color::Rgb(20, 30, 45);
    pub const BG_INPUT_FIELD: Color = Color::Rgb(30, 40, 60);

    // Style presets
    pub fn table_header() -> Style {
        Style::default()
            .fg(Self::SECONDARY)
            .add_modifier(Modifier::BOLD)
    }

    pub fn selected_row() -> Style {
        Style::default()
            .bg(Self::BG_ROW_SELECTED)
            .add_modifier(Modifier::BOLD)
    }

    pub fn bold_white() -> Style {
        Style::default().fg(Self::TEXT).add_modifier(Modifier::BOLD)
    }

    pub fn bold_cyan() -> Style {
        Style::default()
            .fg(Self::PRIMARY)
            .add_modifier(Modifier::BOLD)
    }

    pub fn bold_yellow() -> Style {
        Style::default()
            .fg(Self::SECONDARY)
            .add_modifier(Modifier::BOLD)
    }

    pub fn bold_green() -> Style {
        Style::default()
            .fg(Self::ACCENT_GREEN)
            .add_modifier(Modifier::BOLD)
    }

    pub fn bold_magenta() -> Style {
        Style::default()
            .fg(Self::ACCENT_MAGENTA)
            .add_modifier(Modifier::BOLD)
    }

    pub fn bold_red() -> Style {
        Style::default()
            .fg(Self::DANGER)
            .add_modifier(Modifier::BOLD)
    }

    pub fn muted() -> Style {
        Style::default().fg(Self::MUTED)
    }
}

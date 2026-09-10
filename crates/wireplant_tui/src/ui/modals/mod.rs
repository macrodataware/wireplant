pub mod fault_menu;
pub mod help;
pub mod input;
pub mod search;
pub mod shelve;

use ratatui::Frame;

use crate::app::{AppMode, TuiApp};

/// Dispatches rendering for whichever modal dialog is currently active.
pub fn draw_active_modal(f: &mut Frame, app: &TuiApp) {
    match &app.mode {
        AppMode::ValueInputModal {
            tag_id,
            input_buffer,
            duration_buffer,
            is_discrete,
        } => {
            input::draw(f, tag_id, input_buffer, duration_buffer, *is_discrete);
        }
        AppMode::FaultProfileMenu { selected_index } => {
            fault_menu::draw(f, app, *selected_index);
        }
        AppMode::ShelveAlarmModal {
            alarm_id,
            duration_buffer,
        } => {
            shelve::draw(f, alarm_id, duration_buffer);
        }
        AppMode::TagSearchModal { query_buffer } => {
            search::draw(f, query_buffer);
        }
        AppMode::HelpOverlay => {
            help::draw(f);
        }
        AppMode::Normal => {}
    }
}

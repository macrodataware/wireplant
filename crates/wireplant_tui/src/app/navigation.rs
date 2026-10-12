use wireplant_config::schema::TagConfig;
use wireplant_core::types::MemoryArea;

use super::types::{ActiveTab, TagAreaFilter};
use super::TuiApp;

/// Pure helper to step a selection index within `[0, total - 1]`.
pub fn clamp_step(current: usize, delta: isize, total: usize) -> usize {
    if total == 0 {
        return 0;
    }
    let max_idx = total.saturating_sub(1);
    if delta < 0 {
        current.saturating_sub(delta.unsigned_abs())
    } else {
        (current + delta as usize).min(max_idx)
    }
}

impl TuiApp {
    /// Moves to the next tab in circular order.
    pub fn tab_next(&mut self) {
        self.active_tab = self.active_tab.next();
    }

    /// Moves to the previous tab in circular order.
    pub fn tab_prev(&mut self) {
        self.active_tab = self.active_tab.prev();
    }

    /// Sets the active tab directly.
    pub fn set_tab(&mut self, tab: ActiveTab) {
        self.active_tab = tab;
    }

    /// Returns the filtered list of tags based on current search query and area filter.
    pub fn filtered_tags(&self) -> Vec<&TagConfig> {
        let query = self.tag_search_query.to_lowercase();
        self.scenario
            .tags
            .iter()
            .filter(|t| match self.tag_area_filter {
                TagAreaFilter::All => true,
                TagAreaFilter::HoldingRegister => t.area == MemoryArea::HoldingRegister,
                TagAreaFilter::InputRegister => t.area == MemoryArea::InputRegister,
                TagAreaFilter::Coil => t.area == MemoryArea::Coil,
                TagAreaFilter::DiscreteInput => t.area == MemoryArea::DiscreteInput,
            })
            .filter(|t| {
                if query.is_empty() {
                    return true;
                }
                t.id.to_lowercase().contains(&query)
                    || t.description
                        .as_deref()
                        .map(|d| d.to_lowercase().contains(&query))
                        .unwrap_or(false)
            })
            .collect()
    }

    pub fn select_up(&mut self) {
        let total = self.filtered_tags().len();
        self.selected_tag_index = clamp_step(self.selected_tag_index, -1, total);
        self.tag_table_state.select(Some(self.selected_tag_index));
    }

    pub fn select_down(&mut self) {
        let total = self.filtered_tags().len();
        self.selected_tag_index = clamp_step(self.selected_tag_index, 1, total);
        self.tag_table_state.select(Some(self.selected_tag_index));
    }

    pub fn select_page_down(&mut self) {
        let total = self.filtered_tags().len();
        self.selected_tag_index = clamp_step(self.selected_tag_index, 10, total);
        self.tag_table_state.select(Some(self.selected_tag_index));
    }

    pub fn select_page_up(&mut self) {
        let total = self.filtered_tags().len();
        self.selected_tag_index = clamp_step(self.selected_tag_index, -10, total);
        self.tag_table_state.select(Some(self.selected_tag_index));
    }

    pub fn selected_tag(&self) -> Option<&TagConfig> {
        self.filtered_tags().get(self.selected_tag_index).copied()
    }
}

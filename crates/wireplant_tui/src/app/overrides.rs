use wireplant_core::codec::BOOL_THRESHOLD;

use super::types::{ActiveOverride, AppMode, LogLevel};
use super::{format_value, is_discrete_area, TuiApp};

impl TuiApp {
    /// Opens the override value modal for the currently selected tag.
    pub fn open_override_modal(&mut self) {
        let Some(tag) = self.selected_tag() else {
            return;
        };

        let is_discrete = is_discrete_area(tag.area);
        let current = self
            .active_overrides
            .get(&tag.id)
            .map(|o| o.value)
            .or_else(|| self.sparkline_data.get(&tag.id)?.back().copied());

        self.mode = AppMode::ValueInputModal {
            tag_id: tag.id.clone(),
            input_buffer: format_value(current, is_discrete),
            duration_buffer: String::new(),
            is_discrete,
        };
    }

    /// Toggles the selected tag if it is a Coil or DiscreteInput.
    pub fn toggle_discrete(&mut self) {
        let Some(tag) = self.selected_tag() else {
            return;
        };
        if !is_discrete_area(tag.area) {
            return;
        }

        let tag_id = tag.id.clone();
        let area = tag.area;
        let address = tag.address as usize;

        let current = match self.active_overrides.get(&tag_id) {
            Some(o) => o.value >= BOOL_THRESHOLD,
            None => self.read_bit(area, address),
        };
        let next = !current;
        let value = if next { 1.0 } else { 0.0 };

        match self.write_tag(&tag_id, value) {
            Ok(()) => {
                self.pin_override(&tag_id, value, None);
                self.push_sparkline(&tag_id, value);
                self.add_log(
                    LogLevel::Warn,
                    format!(
                        "Toggled discrete tag '{}' to {}",
                        tag_id,
                        if next { "ON (1)" } else { "OFF (0)" }
                    ),
                );
            }
            Err(reason) => self.add_log(
                LogLevel::Error,
                format!("Could not toggle '{tag_id}': {reason}"),
            ),
        }
    }

    /// Freezes the selected tag at its current sensor reading.
    pub fn freeze_selected_tag(&mut self) {
        let Some(tag) = self.selected_tag() else {
            return;
        };
        let tag_id = tag.id.clone();
        let current_val = self
            .active_overrides
            .get(&tag_id)
            .map(|o| o.value)
            .or_else(|| self.sparkline_data.get(&tag_id)?.back().copied())
            .unwrap_or(0.0);

        self.inject_timed_override(&tag_id, current_val, None);
        self.add_log(
            LogLevel::Warn,
            format!("Tag '{tag_id}' frozen at reading {current_val:.2}"),
        );
    }

    /// Injects a timed override.
    pub fn inject_timed_override(&mut self, tag_id: &str, value: f64, duration_secs: Option<f64>) {
        let deadline = duration_secs.map(|d| self.start_time.elapsed().as_secs_f64() + d);
        match self.write_tag(tag_id, value) {
            Ok(()) => {
                self.pin_override(tag_id, value, deadline);
                self.push_sparkline(tag_id, value);
                let dur_str = match duration_secs {
                    Some(d) => format!("for {d:.1}s"),
                    None => "until cleared".to_string(),
                };
                self.add_log(
                    LogLevel::Warn,
                    format!("Manual override applied to '{tag_id}' = {value:.2} ({dur_str})"),
                );
            }
            Err(e) => {
                self.add_log(
                    LogLevel::Error,
                    format!("Failed to override '{tag_id}': {e}"),
                );
            }
        }
    }

    /// Commits the value in `ValueInputModal`.
    pub fn commit_override_modal(&mut self) {
        let AppMode::ValueInputModal {
            ref tag_id,
            ref input_buffer,
            ref duration_buffer,
            is_discrete,
        } = self.mode
        else {
            return;
        };
        let tag_id = tag_id.clone();
        let input_buffer = input_buffer.clone();
        let duration_buffer = duration_buffer.clone();

        let value = match input_buffer.trim().parse::<f64>() {
            Ok(value) => value,
            Err(_) => {
                self.add_log(
                    LogLevel::Error,
                    format!("Invalid numeric value '{input_buffer}' for tag '{tag_id}'"),
                );
                self.mode = AppMode::Normal;
                return;
            }
        };

        let duration_secs = match duration_buffer.trim().parse::<f64>() {
            Ok(s) if s > 0.0 => Some(s),
            _ => None,
        };

        let units = self
            .scenario
            .tags
            .iter()
            .find(|t| t.id == tag_id)
            .map(|t| t.units.clone())
            .unwrap_or_default();

        let deadline = duration_secs.map(|d| self.start_time.elapsed().as_secs_f64() + d);

        match self.write_tag(&tag_id, value) {
            Ok(()) => {
                self.pin_override(&tag_id, value, deadline);
                self.push_sparkline(&tag_id, value);
                let shown = if is_discrete {
                    if value >= BOOL_THRESHOLD {
                        "ON (1)".to_string()
                    } else {
                        "OFF (0)".to_string()
                    }
                } else {
                    format!("{value:.2} {units}")
                };
                let dur_note = match duration_secs {
                    Some(d) => format!(" (hold for {d:.0}s)"),
                    None => String::new(),
                };
                self.add_log(
                    LogLevel::Warn,
                    format!("Manual override applied to '{tag_id}' = {shown}{dur_note}"),
                );
            }
            Err(reason) => self.add_log(
                LogLevel::Error,
                format!("Could not override '{tag_id}': {reason}"),
            ),
        }

        self.mode = AppMode::Normal;
    }

    /// Clears the override on the currently selected tag.
    pub fn clear_override_selected(&mut self) {
        let Some(tag) = self.selected_tag() else {
            return;
        };
        let id = tag.id.clone();
        if let Ok(mut reg) = self.overrides.try_write() {
            reg.remove_override(&id);
        }
        if self.active_overrides.remove(&id).is_some() {
            self.add_log(
                LogLevel::Info,
                format!("Cleared manual override on tag '{id}'"),
            );
        }
    }

    /// Clears all active tag overrides.
    pub fn clear_all_overrides(&mut self) {
        if let Ok(mut reg) = self.overrides.try_write() {
            reg.clear_all();
        }
        let count = self.active_overrides.len();
        self.active_overrides.clear();
        self.add_log(
            LogLevel::Info,
            format!("Cleared all {count} active tag overrides"),
        );
    }

    /// Triggers a predefined fault profile by index.
    pub fn trigger_fault_profile(&mut self, profile_index: usize) {
        let Some(profile) = self.scenario.fault_profiles.get(profile_index).cloned() else {
            return;
        };

        let now = self.start_time.elapsed().as_secs_f64();
        let mut applied = 0usize;
        for action in &profile.actions {
            let deadline = action.duration_secs.map(|d| now + d);
            match self.write_tag(&action.target_tag, action.force_value) {
                Ok(()) => {
                    self.pin_override(&action.target_tag, action.force_value, deadline);
                    self.push_sparkline(&action.target_tag, action.force_value);
                    applied += 1;
                }
                Err(reason) => self.add_log(
                    LogLevel::Error,
                    format!("Could not inject '{}': {reason}", action.target_tag),
                ),
            }
        }

        self.add_log(
            LogLevel::Warn,
            format!(
                "Triggered fault profile '{}' ({applied} of {} actions injected)",
                profile.name,
                profile.actions.len()
            ),
        );
    }

    pub(crate) fn pin_override(&mut self, tag_id: &str, value: f64, expires_at_secs: Option<f64>) {
        if let Ok(mut reg) = self.overrides.try_write() {
            reg.set_override(tag_id.to_string(), value, expires_at_secs);
        }
        self.active_overrides.insert(
            tag_id.to_string(),
            ActiveOverride {
                value,
                expires_at_secs,
            },
        );
    }

    pub(crate) fn expire_overrides(&mut self, elapsed_secs: f64) {
        let expired: Vec<String> = self
            .active_overrides
            .iter()
            .filter(|(_, o)| o.expires_at_secs.is_some_and(|at| at <= elapsed_secs))
            .map(|(id, _)| id.clone())
            .collect();

        for id in expired {
            if let Ok(mut reg) = self.overrides.try_write() {
                reg.remove_override(&id);
            }
            self.active_overrides.remove(&id);
            self.add_log(LogLevel::Info, format!("Override on '{id}' expired"));
        }
    }
}

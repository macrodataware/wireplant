use wireplant_engine::alarm::{ActiveAlarm, IsaAlarmState};

use super::types::{AppMode, LogLevel};
use super::TuiApp;

impl TuiApp {
    pub(crate) fn update_alarm_metrics(&mut self) {
        self.active_alarms_count = self
            .cached_alarms
            .iter()
            .filter(|a| a.condition_active)
            .count();
        self.unacknowledged_alarms_count = self
            .cached_alarms
            .iter()
            .filter(|a| {
                matches!(
                    a.state,
                    IsaAlarmState::Unacknowledged | IsaAlarmState::ReturnToNormalUnack
                )
            })
            .count();
    }

    /// Pulls current alarm states from the manager without blocking the event thread.
    pub fn sync_alarms_from_inner(&mut self) {
        let snapshot = self.alarms.inner().try_read().map(|g| g.clone());
        if let Ok(alarms) = snapshot {
            self.cached_alarms = alarms;
            self.update_alarm_metrics();
        }
    }

    /// Refreshes cached alarm states asynchronously.
    pub async fn refresh_alarms(&mut self) {
        self.cached_alarms = self.alarms.get_snapshot().await;
        self.update_alarm_metrics();
    }

    pub fn selected_alarm(&self) -> Option<&ActiveAlarm> {
        let idx = self.alarm_table_state.selected().unwrap_or(0);
        self.cached_alarms.get(idx)
    }

    pub fn alarm_select_up(&mut self) {
        let total = self.cached_alarms.len();
        let current = self.alarm_table_state.selected().unwrap_or(0);
        let next = super::navigation::clamp_step(current, -1, total);
        self.alarm_table_state.select(Some(next));
    }

    pub fn alarm_select_down(&mut self) {
        let total = self.cached_alarms.len();
        let current = self.alarm_table_state.selected().unwrap_or(0);
        let next = super::navigation::clamp_step(current, 1, total);
        self.alarm_table_state.select(Some(next));
    }

    /// Acknowledge the selected alarm.
    pub async fn acknowledge_selected_alarm(&mut self) {
        let Some(alarm) = self.selected_alarm().cloned() else {
            return;
        };
        match self.alarms.acknowledge(&alarm.config.id).await {
            Ok(new_state) => {
                self.add_log(
                    LogLevel::Info,
                    format!(
                        "Alarm '{}' acknowledged -> {:?}",
                        alarm.config.id, new_state
                    ),
                );
                self.refresh_alarms().await;
            }
            Err(e) => {
                self.add_log(
                    LogLevel::Error,
                    format!("Cannot ack alarm '{}': {e}", alarm.config.id),
                );
            }
        }
    }

    /// Reset the selected alarm (if latched and condition normal).
    pub async fn reset_selected_alarm(&mut self) {
        let Some(alarm) = self.selected_alarm().cloned() else {
            return;
        };
        match self.alarms.reset(&alarm.config.id).await {
            Ok(new_state) => {
                self.add_log(
                    LogLevel::Info,
                    format!("Alarm '{}' reset -> {:?}", alarm.config.id, new_state),
                );
                self.refresh_alarms().await;
            }
            Err(e) => {
                self.add_log(
                    LogLevel::Error,
                    format!("Cannot reset alarm '{}': {e}", alarm.config.id),
                );
            }
        }
    }

    /// Opens the shelve duration modal for the selected alarm.
    pub fn open_shelve_modal(&mut self) {
        let Some(alarm) = self.selected_alarm() else {
            return;
        };
        self.mode = AppMode::ShelveAlarmModal {
            alarm_id: alarm.config.id.clone(),
            duration_buffer: "300".to_string(),
        };
    }

    /// Commits the shelve duration.
    pub async fn commit_shelve_modal(&mut self) {
        let AppMode::ShelveAlarmModal {
            ref alarm_id,
            ref duration_buffer,
        } = self.mode
        else {
            return;
        };
        let alarm_id = alarm_id.clone();
        let trimmed = duration_buffer.trim();
        let secs = match trimmed.parse::<u64>() {
            Ok(s) if s > 0 && s <= 86400 * 365 => s,
            _ => {
                self.add_log(
                    LogLevel::Error,
                    "Invalid shelving duration: must be between 1 and 31536000 seconds",
                );
                self.mode = AppMode::Normal;
                return;
            }
        };

        match self
            .alarms
            .shelve(&alarm_id, std::time::Duration::from_secs(secs))
            .await
        {
            Ok(_) => {
                self.add_log(
                    LogLevel::Warn,
                    format!("Alarm '{alarm_id}' shelved for {secs}s"),
                );
                self.refresh_alarms().await;
            }
            Err(e) => {
                self.add_log(
                    LogLevel::Error,
                    format!("Failed to shelve '{alarm_id}': {e}"),
                );
            }
        }
        self.mode = AppMode::Normal;
    }

    /// Unshelves the selected alarm immediately.
    pub async fn unshelve_selected_alarm(&mut self) {
        let Some(alarm) = self.selected_alarm().cloned() else {
            return;
        };
        match self.alarms.unshelve(&alarm.config.id).await {
            Ok(new_state) => {
                self.add_log(
                    LogLevel::Info,
                    format!("Alarm '{}' unshelved -> {:?}", alarm.config.id, new_state),
                );
                self.refresh_alarms().await;
            }
            Err(e) => {
                self.add_log(
                    LogLevel::Error,
                    format!("Cannot unshelve alarm '{}': {e}", alarm.config.id),
                );
            }
        }
    }
}

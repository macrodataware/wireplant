//! ANSI/ISA-18.2 compliant alarm state engine for industrial automation.
//!
//! Provides alarm state machine lifecycle transitions (Normal, Unacknowledged,
//! Acknowledged, ReturnToNormalUnack, Latched, Shelved), deadband hysteresis to
//! eliminate chattering, on/off-delay qualification filters, and bidirectional
//! synchronization with Modbus coils and registers.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, RwLock};
use wireplant_core::types::MemoryArea;

/// Severity classification aligned with ISA-18.2 alarm management.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlarmSeverity {
    Low,
    #[default]
    Medium,
    High,
    Critical,
}

impl AlarmSeverity {
    pub const fn label(self) -> &'static str {
        match self {
            AlarmSeverity::Critical => "CRITICAL",
            AlarmSeverity::High => "HIGH",
            AlarmSeverity::Medium => "MEDIUM",
            AlarmSeverity::Low => "LOW",
        }
    }
}

/// Alarm limit comparison direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlarmType {
    High,
    Low,
    HighHigh,
    LowLow,
}

/// Domain errors produced during alarm lifecycle state transitions.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AlarmError {
    #[error("Alarm '{0}' not found")]
    NotFound(String),
    #[error("Invalid alarm transition: {0}")]
    InvalidTransition(String),
}

/// ANSI/ISA-18.2 standardized alarm lifecycle states.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u16)]
#[serde(rename_all = "snake_case")]
pub enum IsaAlarmState {
    Normal = 0,
    Unacknowledged = 1,
    Acknowledged = 2,
    ReturnToNormalUnack = 3,
    Latched = 4,
    Shelved = 5,
}

/// Configuration defining an ISA-18.2 alarm rule and its SCADA memory mapping.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AlarmConfig {
    pub id: String,
    #[serde(default)]
    pub description: String,
    pub source_tag: String,
    pub alarm_type: AlarmType,
    pub limit: f64,
    #[serde(default)]
    pub deadband: f64,
    #[serde(default)]
    pub severity: AlarmSeverity,
    #[serde(default)]
    pub on_delay_secs: f64,
    #[serde(default)]
    pub off_delay_secs: f64,
    #[serde(default)]
    pub latching: bool,
    #[serde(default = "AlarmConfig::default_max_shelve_duration")]
    pub max_shelve_duration_secs: f64,

    // Process Image Address Mapping
    #[serde(default)]
    pub active_coil_addr: Option<usize>,
    #[serde(default)]
    pub unack_coil_addr: Option<usize>,
    #[serde(default)]
    pub ack_coil_addr: Option<usize>,
    #[serde(default)]
    pub reset_coil_addr: Option<usize>,
    #[serde(default)]
    pub state_reg_addr: Option<usize>,
}

impl AlarmConfig {
    pub const DEFAULT_MAX_SHELVE_DURATION_SECS: f64 = 86400.0; // 24 hours

    const fn default_max_shelve_duration() -> f64 {
        Self::DEFAULT_MAX_SHELVE_DURATION_SECS
    }
}

/// Runtime instance of an active ISA-18.2 alarm tracking state transitions and timers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveAlarm {
    pub config: AlarmConfig,
    pub state: IsaAlarmState,
    #[serde(skip)]
    pub on_delay_timer: Duration,
    #[serde(skip)]
    pub off_delay_timer: Duration,
    #[serde(skip)]
    pub shelve_timer: Duration,
    #[serde(skip)]
    pub pre_shelve_state: Option<IsaAlarmState>,
    pub condition_active: bool,
    pub current_value: f64,
    pub last_trip_timestamp_secs: f64,
}

impl ActiveAlarm {
    pub fn new(config: AlarmConfig) -> Self {
        Self {
            config,
            state: IsaAlarmState::Normal,
            on_delay_timer: Duration::ZERO,
            off_delay_timer: Duration::ZERO,
            shelve_timer: Duration::ZERO,
            pre_shelve_state: None,
            condition_active: false,
            current_value: 0.0,
            last_trip_timestamp_secs: 0.0,
        }
    }

    /// Evaluates if the process value exceeds the alarm limit taking deadband hysteresis into account.
    pub fn evaluate_condition(&self, value: f64) -> bool {
        match self.config.alarm_type {
            AlarmType::High | AlarmType::HighHigh => {
                if self.condition_active {
                    // Stays active until process drops strictly below limit minus deadband
                    value >= (self.config.limit - self.config.deadband)
                } else {
                    value >= self.config.limit
                }
            }
            AlarmType::Low | AlarmType::LowLow => {
                if self.condition_active {
                    // Stays active until process rises strictly below limit plus deadband
                    value <= (self.config.limit + self.config.deadband)
                } else {
                    value <= self.config.limit
                }
            }
        }
    }

    /// Advances qualification timers and performs ISA-18.2 state transitions for one tick.
    pub fn step(&mut self, source_val: f64, dt: Duration, current_time_secs: f64) {
        self.current_value = source_val;
        let raw_condition = self.evaluate_condition(source_val);

        if raw_condition {
            self.on_delay_timer += dt;
            self.off_delay_timer = Duration::ZERO;
        } else {
            self.off_delay_timer += dt;
            self.on_delay_timer = Duration::ZERO;
        }

        let on_delay_threshold = Duration::from_secs_f64(self.config.on_delay_secs.max(0.0));
        let off_delay_threshold = Duration::from_secs_f64(self.config.off_delay_secs.max(0.0));

        let qualified_condition = if raw_condition && self.on_delay_timer >= on_delay_threshold {
            true
        } else if !raw_condition && self.off_delay_timer >= off_delay_threshold {
            false
        } else {
            self.condition_active
        };

        self.condition_active = qualified_condition;

        // State Machine Transition Logic
        match self.state {
            IsaAlarmState::Normal => {
                if self.condition_active {
                    self.state = IsaAlarmState::Unacknowledged;
                    self.last_trip_timestamp_secs = current_time_secs;
                }
            }
            IsaAlarmState::Unacknowledged => {
                if !self.condition_active {
                    self.state = IsaAlarmState::ReturnToNormalUnack;
                }
            }
            IsaAlarmState::Acknowledged => {
                if !self.condition_active {
                    if self.config.latching {
                        self.state = IsaAlarmState::Latched;
                    } else {
                        self.state = IsaAlarmState::Normal;
                    }
                }
            }
            IsaAlarmState::ReturnToNormalUnack => {
                if self.condition_active {
                    self.state = IsaAlarmState::Unacknowledged;
                }
            }
            IsaAlarmState::Latched => {
                if self.condition_active {
                    self.state = IsaAlarmState::Unacknowledged;
                }
            }
            IsaAlarmState::Shelved => {
                if self.shelve_timer > dt {
                    self.shelve_timer -= dt;
                } else {
                    self.shelve_timer = Duration::ZERO;
                    let pre_shelve = self
                        .pre_shelve_state
                        .take()
                        .unwrap_or(IsaAlarmState::Normal);
                    if self.condition_active {
                        self.state = IsaAlarmState::Unacknowledged;
                    } else if self.config.latching
                        && matches!(
                            pre_shelve,
                            IsaAlarmState::Latched
                                | IsaAlarmState::Acknowledged
                                | IsaAlarmState::Unacknowledged
                                | IsaAlarmState::ReturnToNormalUnack
                        )
                    {
                        // Latched alarm requires explicit operator reset per ISA-18.2
                        self.state = IsaAlarmState::Latched;
                    } else {
                        self.state = IsaAlarmState::Normal;
                    }
                }
            }
        }
    }

    /// Operator acknowledges the alarm.
    pub fn acknowledge(&mut self) {
        match self.state {
            IsaAlarmState::Unacknowledged => {
                self.state = IsaAlarmState::Acknowledged;
            }
            IsaAlarmState::ReturnToNormalUnack => {
                if self.config.latching {
                    self.state = IsaAlarmState::Latched;
                } else {
                    self.state = IsaAlarmState::Normal;
                }
            }
            _ => {}
        }
    }

    /// Operator resets a latched alarm once the physical condition has returned to normal.
    pub fn reset(&mut self) -> Result<(), &'static str> {
        if self.state == IsaAlarmState::Latched {
            if !self.condition_active {
                self.state = IsaAlarmState::Normal;
                Ok(())
            } else {
                Err("Cannot reset while condition is still active")
            }
        } else {
            Ok(())
        }
    }

    /// Temporarily shelves the alarm for up to `duration`.
    pub fn shelve(&mut self, duration: Duration) {
        if self.state != IsaAlarmState::Shelved {
            self.pre_shelve_state = Some(self.state);
        }
        let max_secs = if self.config.max_shelve_duration_secs.is_finite()
            && self.config.max_shelve_duration_secs > 0.0
        {
            self.config.max_shelve_duration_secs
        } else {
            AlarmConfig::DEFAULT_MAX_SHELVE_DURATION_SECS
        };
        let max_duration =
            Duration::try_from_secs_f64(max_secs).unwrap_or(Duration::from_secs(86400));
        self.shelve_timer = if max_duration.is_zero() {
            duration
        } else {
            duration.min(max_duration)
        };
        self.state = IsaAlarmState::Shelved;
    }

    /// Immediately unshelves the alarm, returning to active monitoring.
    pub fn unshelve(&mut self) {
        if self.state == IsaAlarmState::Shelved {
            self.shelve_timer = Duration::ZERO;
            let pre_shelve = self
                .pre_shelve_state
                .take()
                .unwrap_or(IsaAlarmState::Normal);
            if self.condition_active {
                self.state = IsaAlarmState::Unacknowledged;
            } else if self.config.latching
                && matches!(
                    pre_shelve,
                    IsaAlarmState::Latched
                        | IsaAlarmState::Acknowledged
                        | IsaAlarmState::Unacknowledged
                        | IsaAlarmState::ReturnToNormalUnack
                )
            {
                self.state = IsaAlarmState::Latched;
            } else {
                self.state = IsaAlarmState::Normal;
            }
        }
    }

    /// Remaining seconds if shelved, or 0.0.
    pub fn shelved_remaining_secs(&self) -> f64 {
        if self.state == IsaAlarmState::Shelved {
            self.shelve_timer.as_secs_f64()
        } else {
            0.0
        }
    }
}

/// Lifecycle transition event emitted whenever an alarm changes state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AlarmTransitionEvent {
    pub timestamp_ms: i64,
    pub alarm_id: String,
    pub previous_state: IsaAlarmState,
    pub new_state: IsaAlarmState,
    pub value: Option<f64>,
    pub message: String,
}

/// Thread-safe manager coordinating active alarms between simulation ticks and API/SCADA clients.
#[derive(Clone, Debug)]
pub struct AlarmManager {
    alarms: Arc<RwLock<Vec<ActiveAlarm>>>,
    transition_tx: broadcast::Sender<AlarmTransitionEvent>,
    current_sim_time_ms: Arc<AtomicI64>,
}

impl Default for AlarmManager {
    fn default() -> Self {
        let (transition_tx, _) = broadcast::channel(512);
        Self {
            alarms: Arc::new(RwLock::new(Vec::new())),
            transition_tx,
            current_sim_time_ms: Arc::new(AtomicI64::new(0)),
        }
    }
}

impl AlarmManager {
    pub fn new(alarms: Vec<ActiveAlarm>) -> Self {
        let (transition_tx, _) = broadcast::channel(512);
        Self {
            alarms: Arc::new(RwLock::new(alarms)),
            transition_tx,
            current_sim_time_ms: Arc::new(AtomicI64::new(0)),
        }
    }

    pub fn from_configs(configs: &[AlarmConfig]) -> Self {
        let active = configs.iter().cloned().map(ActiveAlarm::new).collect();
        Self::new(active)
    }

    /// Resolves the current simulation timestamp in milliseconds.
    #[inline]
    pub fn current_time_ms(&self) -> i64 {
        self.current_sim_time_ms.load(Ordering::Acquire)
    }

    /// Explicitly updates the simulation clock (called by step or test harnesses).
    #[inline]
    pub fn set_sim_time_ms(&self, ms: i64) {
        self.current_sim_time_ms.store(ms, Ordering::Release);
    }

    pub fn inner(&self) -> &Arc<RwLock<Vec<ActiveAlarm>>> {
        &self.alarms
    }

    pub fn subscribe_transitions(&self) -> broadcast::Receiver<AlarmTransitionEvent> {
        self.transition_tx.subscribe()
    }

    /// Steps all active alarms forward by one simulation delta.
    pub async fn step(
        &self,
        calculated_values: &HashMap<String, f64>,
        dt: Duration,
        current_time_secs: f64,
    ) {
        let now_ms = (current_time_secs * 1000.0) as i64;
        self.set_sim_time_ms(now_ms);

        let mut list = self.alarms.write().await;

        for alarm in list.iter_mut() {
            let old_state = alarm.state;
            let source_val = calculated_values
                .get(&alarm.config.source_tag)
                .copied()
                .unwrap_or(0.0);
            alarm.step(source_val, dt, current_time_secs);

            if alarm.state != old_state {
                let event = AlarmTransitionEvent {
                    timestamp_ms: now_ms,
                    alarm_id: alarm.config.id.clone(),
                    previous_state: old_state,
                    new_state: alarm.state,
                    value: Some(source_val),
                    message: format!(
                        "Alarm '{}' transitioned from {:?} to {:?}",
                        alarm.config.id, old_state, alarm.state
                    ),
                };
                let _ = self.transition_tx.send(event);
            }
        }
    }

    /// Synchronizes alarm status with the Process Image and processes operator Modbus pulses.
    pub async fn sync_to_process_image(&self, memory: &mut wireplant_core::memory::PlantMemory) {
        let mut list = self.alarms.write().await;
        let now_ms = self.current_time_ms();

        for alarm in list.iter_mut() {
            // Check if operator pulsed Ack Coil via Modbus
            if let Some(addr) = alarm.config.ack_coil_addr {
                if let Ok(true) = memory.read_bit(MemoryArea::Coil, addr) {
                    let old_state = alarm.state;
                    alarm.acknowledge();
                    let _ = memory.write_bit(MemoryArea::Coil, addr, false);
                    if alarm.state != old_state {
                        let event = AlarmTransitionEvent {
                            timestamp_ms: now_ms,
                            alarm_id: alarm.config.id.clone(),
                            previous_state: old_state,
                            new_state: alarm.state,
                            value: Some(alarm.current_value),
                            message: format!(
                                "Modbus pulse acknowledged alarm '{}'",
                                alarm.config.id
                            ),
                        };
                        let _ = self.transition_tx.send(event);
                    }
                }
            }

            // Check if operator pulsed Reset Coil via Modbus
            if let Some(addr) = alarm.config.reset_coil_addr {
                if let Ok(true) = memory.read_bit(MemoryArea::Coil, addr) {
                    let old_state = alarm.state;
                    let _ = alarm.reset();
                    let _ = memory.write_bit(MemoryArea::Coil, addr, false);
                    if alarm.state != old_state {
                        let event = AlarmTransitionEvent {
                            timestamp_ms: now_ms,
                            alarm_id: alarm.config.id.clone(),
                            previous_state: old_state,
                            new_state: alarm.state,
                            value: Some(alarm.current_value),
                            message: format!("Modbus pulse reset alarm '{}'", alarm.config.id),
                        };
                        let _ = self.transition_tx.send(event);
                    }
                }
            }

            // Active condition coil
            if let Some(addr) = alarm.config.active_coil_addr {
                let _ = memory.write_bit(MemoryArea::Coil, addr, alarm.condition_active);
            }

            // Unacknowledged coil (flashing annunciator lamp)
            if let Some(addr) = alarm.config.unack_coil_addr {
                let is_unack = matches!(
                    alarm.state,
                    IsaAlarmState::Unacknowledged | IsaAlarmState::ReturnToNormalUnack
                );
                let _ = memory.write_bit(MemoryArea::Coil, addr, is_unack);
            }

            // State holding register (0 to 5)
            if let Some(addr) = alarm.config.state_reg_addr {
                let state_val = alarm.state as u16;
                let _ =
                    memory.write_registers_slice(MemoryArea::HoldingRegister, addr, &[state_val]);
            }
        }
    }

    async fn mutate_alarm<F>(
        &self,
        id: &str,
        message_desc: &str,
        f: F,
    ) -> Result<IsaAlarmState, AlarmError>
    where
        F: FnOnce(&mut ActiveAlarm) -> Result<(), AlarmError>,
    {
        let mut list = self.alarms.write().await;
        let alarm = list
            .iter_mut()
            .find(|a| a.config.id == id)
            .ok_or_else(|| AlarmError::NotFound(id.to_string()))?;
        let old_state = alarm.state;
        f(alarm)?;
        if alarm.state != old_state {
            let now_ms = self.current_time_ms();
            let event = AlarmTransitionEvent {
                timestamp_ms: now_ms,
                alarm_id: alarm.config.id.clone(),
                previous_state: old_state,
                new_state: alarm.state,
                value: Some(alarm.current_value),
                message: format!("Operator {message_desc} alarm '{}'", id),
            };
            let _ = self.transition_tx.send(event);
        }
        Ok(alarm.state)
    }

    pub async fn acknowledge(&self, id: &str) -> Result<IsaAlarmState, AlarmError> {
        self.mutate_alarm(id, "acknowledged", |a| {
            a.acknowledge();
            Ok(())
        })
        .await
    }

    pub async fn reset(&self, id: &str) -> Result<IsaAlarmState, AlarmError> {
        self.mutate_alarm(id, "reset latched", |a| {
            a.reset()
                .map_err(|e| AlarmError::InvalidTransition(e.to_string()))
        })
        .await
    }

    pub async fn shelve(&self, id: &str, duration: Duration) -> Result<IsaAlarmState, AlarmError> {
        self.mutate_alarm(id, "shelved", |a| {
            a.shelve(duration);
            Ok(())
        })
        .await
    }

    pub async fn unshelve(&self, id: &str) -> Result<IsaAlarmState, AlarmError> {
        self.mutate_alarm(id, "unshelved", |a| {
            a.unshelve();
            Ok(())
        })
        .await
    }

    pub async fn get_snapshot(&self) -> Vec<ActiveAlarm> {
        self.alarms.read().await.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config(limit: f64, deadband: f64, latching: bool) -> AlarmConfig {
        AlarmConfig {
            id: "high_press".into(),
            description: "High Pressure Alarm".into(),
            source_tag: "pressure".into(),
            alarm_type: AlarmType::High,
            limit,
            deadband,
            severity: AlarmSeverity::High,
            on_delay_secs: 0.0,
            off_delay_secs: 0.0,
            latching,
            max_shelve_duration_secs: 3600.0,
            active_coil_addr: Some(10),
            unack_coil_addr: Some(11),
            ack_coil_addr: Some(12),
            reset_coil_addr: Some(13),
            state_reg_addr: Some(50),
        }
    }

    #[test]
    fn deadband_prevents_alarm_chatter_across_threshold() {
        let config = test_config(100.0, 5.0, false);
        let mut alarm = ActiveAlarm::new(config);

        // Below threshold: normal
        alarm.step(99.0, Duration::from_millis(100), 0.1);
        assert_eq!(alarm.state, IsaAlarmState::Normal);
        assert!(!alarm.condition_active);

        // Crosses threshold: unack
        alarm.step(101.0, Duration::from_millis(100), 0.2);
        assert_eq!(alarm.state, IsaAlarmState::Unacknowledged);
        assert!(alarm.condition_active);

        // Drops within deadband (98.0 >= 100.0 - 5.0 = 95.0): remains active!
        alarm.step(98.0, Duration::from_millis(100), 0.3);
        assert_eq!(alarm.state, IsaAlarmState::Unacknowledged);
        assert!(alarm.condition_active);

        // Drops below deadband (94.0 < 95.0): clears condition, moves to ReturnToNormalUnack
        alarm.step(94.0, Duration::from_millis(100), 0.4);
        assert_eq!(alarm.state, IsaAlarmState::ReturnToNormalUnack);
        assert!(!alarm.condition_active);

        // Operator acknowledges: returns to Normal
        alarm.acknowledge();
        assert_eq!(alarm.state, IsaAlarmState::Normal);
    }

    #[test]
    fn on_delay_qualifies_alarm_only_after_persisting() {
        let mut config = test_config(100.0, 0.0, false);
        config.on_delay_secs = 1.0;
        let mut alarm = ActiveAlarm::new(config);

        // Transient spike for 0.5s: does NOT trip
        alarm.step(105.0, Duration::from_millis(500), 0.5);
        assert_eq!(alarm.state, IsaAlarmState::Normal);
        assert!(!alarm.condition_active);

        // Process drops back: timer resets
        alarm.step(90.0, Duration::from_millis(200), 0.7);
        assert_eq!(alarm.state, IsaAlarmState::Normal);
        assert!(!alarm.condition_active);

        // Spike for full 1.0s: trips
        alarm.step(105.0, Duration::from_millis(1000), 1.7);
        assert_eq!(alarm.state, IsaAlarmState::Unacknowledged);
        assert!(alarm.condition_active);
    }

    #[test]
    fn latching_alarm_remains_active_until_operator_reset() {
        let config = test_config(100.0, 2.0, true);
        let mut alarm = ActiveAlarm::new(config);

        // Trip alarm
        alarm.step(102.0, Duration::from_millis(100), 0.1);
        assert_eq!(alarm.state, IsaAlarmState::Unacknowledged);

        // Acknowledge while still in alarm
        alarm.acknowledge();
        assert_eq!(alarm.state, IsaAlarmState::Acknowledged);

        // Process returns to normal: latches!
        alarm.step(95.0, Duration::from_millis(100), 0.2);
        assert_eq!(alarm.state, IsaAlarmState::Latched);
        assert!(!alarm.condition_active);

        // Reset command restores Normal
        assert!(alarm.reset().is_ok());
        assert_eq!(alarm.state, IsaAlarmState::Normal);
    }

    #[test]
    fn shelving_suppresses_alarm_until_timeout_or_operator_unshelve() {
        let config = test_config(100.0, 2.0, false);
        let mut alarm = ActiveAlarm::new(config);

        alarm.shelve(Duration::from_secs(60));
        assert_eq!(alarm.state, IsaAlarmState::Shelved);

        // While shelved, even if limit exceeded, stays shelved
        alarm.step(110.0, Duration::from_secs(10), 10.0);
        assert_eq!(alarm.state, IsaAlarmState::Shelved);

        // Unshelve resumes monitoring, immediately seeing active condition
        alarm.unshelve();
        assert_eq!(alarm.state, IsaAlarmState::Unacknowledged);
    }

    #[tokio::test]
    async fn alarm_mutations_share_simulation_time_domain() {
        let mgr = AlarmManager::from_configs(&[test_config(100.0, 2.0, false)]);
        let mut rx = mgr.subscribe_transitions();

        // Step at simulation t = 5.0s (5000ms) with pressure exceeding limit
        let mut vals = HashMap::new();
        vals.insert("pressure".to_string(), 150.0);
        mgr.step(&vals, Duration::from_millis(100), 5.0).await;

        let trip_event = rx.recv().await.unwrap();
        assert_eq!(trip_event.timestamp_ms, 5000);
        assert_eq!(trip_event.new_state, IsaAlarmState::Unacknowledged);

        // Operator acknowledges via API
        mgr.acknowledge("high_press").await.unwrap();
        let ack_event = rx.recv().await.unwrap();
        assert_eq!(
            ack_event.timestamp_ms, 5000,
            "API mutation must inherit current simulation clock timestamp"
        );
        assert_eq!(ack_event.new_state, IsaAlarmState::Acknowledged);
    }
}

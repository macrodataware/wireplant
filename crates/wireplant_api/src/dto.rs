use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub uptime_secs: u64,
    pub facility_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct MemorySummary {
    pub coils_total: usize,
    pub holding_registers_total: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlantStateResponse {
    pub facility_name: String,
    /// Open Modbus TCP sessions, as counted by the listener.
    pub modbus_sessions: usize,
    pub active_tags: HashMap<String, f64>,
    #[serde(default)]
    pub memory: MemorySummary,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub coils_summary: Vec<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub registers_summary: Vec<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FaultInjectionRequest {
    pub tag_id: String,
    pub forced_value: f64,
    #[serde(default)]
    pub duration_secs: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FaultInjectionResponse {
    pub success: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlarmResponse {
    pub id: String,
    pub description: String,
    pub source_tag: String,
    pub alarm_type: wireplant_engine::alarm::AlarmType,
    pub severity: wireplant_engine::alarm::AlarmSeverity,
    pub state: wireplant_engine::alarm::IsaAlarmState,
    pub state_code: u16,
    pub limit: f64,
    pub deadband: f64,
    pub condition_active: bool,
    pub current_value: f64,
    pub last_trip_timestamp_secs: f64,
    pub shelved_remaining_secs: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShelveAlarmRequest {
    pub duration_secs: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlarmActionResponse {
    pub success: bool,
    pub alarm_id: String,
    pub state: wireplant_engine::alarm::IsaAlarmState,
    pub message: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct HistoryQuery {
    pub tag: String,
    pub start_ms: Option<i64>,
    pub end_ms: Option<i64>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryResponsePoint {
    pub timestamp_ms: i64,
    pub value: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryResponse {
    pub tag_id: String,
    pub points: Vec<HistoryResponsePoint>,
}

//! # WirePlant API (`wireplant_api`)
//!
//! Embedded Axum REST API providing live state inspection, manual override and fault
//! injection, alarm lifecycle acknowledgment, and Prometheus telemetry metrics.

pub mod auth;
pub mod dto;
pub mod metrics;
pub mod routes;
pub mod server;

pub use auth::require_auth_middleware;

pub use dto::{
    AlarmActionResponse, AlarmResponse, FaultInjectionRequest, FaultInjectionResponse,
    HealthResponse, HistoryQuery, HistoryResponse, HistoryResponsePoint, MemorySummary,
    PlantStateResponse, ShelveAlarmRequest,
};
pub use routes::{
    acknowledge_alarm_handler, get_alarms_handler, get_state_handler, health_handler,
    inject_fault_handler, query_history_handler, reset_alarm_handler, shelve_alarm_handler,
    unshelve_alarm_handler, AppState, StateQuery,
};
pub use server::{ApiServer, ApiServerBuilder};

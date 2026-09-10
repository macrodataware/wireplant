use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use std::sync::Arc;
use std::time::{Duration, Instant};
use wireplant_config::schema::ScenarioConfig;
use wireplant_core::codec::{decode_tag, encode_tag};
use wireplant_core::memory::{SessionCounter, SharedProcessImage};
use wireplant_core::overrides::SharedOverrides;
use wireplant_engine::alarm::AlarmManager;
use wireplant_engine::historian::Historian;

use crate::dto::{
    AlarmActionResponse, AlarmResponse, FaultInjectionRequest, FaultInjectionResponse,
    HealthResponse, HistoryQuery, HistoryResponse, HistoryResponsePoint, MemorySummary,
    PlantStateResponse, ShelveAlarmRequest,
};

use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug, Default, Deserialize)]
pub struct StateQuery {
    pub include_raw: Option<bool>,
}

#[derive(Debug)]
pub struct AppState {
    pub scenario: ScenarioConfig,
    pub shared_memory: SharedProcessImage,
    pub sessions: SessionCounter,
    pub alarms: AlarmManager,
    pub overrides: SharedOverrides,
    pub historian: Option<Arc<Historian>>,
    pub engine_liveness: Option<Arc<AtomicBool>>,
    pub start_time: Instant,
}

pub async fn health_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let is_alive = match &state.engine_liveness {
        Some(flag) => flag.load(Ordering::Relaxed),
        None => true, // default for tests where liveness handle isn't provided
    };

    let uptime = state.start_time.elapsed().as_secs();

    if is_alive {
        (
            StatusCode::OK,
            Json(HealthResponse {
                status: "ok",
                uptime_secs: uptime,
                facility_name: state.scenario.facility.name.clone(),
            }),
        )
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(HealthResponse {
                status: "degraded",
                uptime_secs: uptime,
                facility_name: state.scenario.facility.name.clone(),
            }),
        )
    }
}

pub async fn get_state_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<StateQuery>,
) -> Result<Json<PlantStateResponse>, ApiError> {
    let mem = state.shared_memory.read().await;
    let mut active_tags = std::collections::HashMap::with_capacity(state.scenario.tags.len());

    for tag in &state.scenario.tags {
        // A tag the schema allowed but the Process Image cannot hold is a server
        // fault, not a value of zero: saying so beats inventing a reading.
        let value = decode_tag(&mem, tag).map_err(|e| ApiError::TagUnreadable {
            tag_id: tag.id.clone(),
            reason: e.to_string(),
        })?;
        active_tags.insert(tag.id.clone(), value);
    }

    let memory = MemorySummary {
        coils_total: mem.coils.len(),
        holding_registers_total: mem.holding_registers.len(),
    };

    let (coils_summary, registers_summary) = if query.include_raw.unwrap_or(false) {
        (mem.coils.clone(), mem.holding_registers.clone())
    } else {
        (Vec::new(), Vec::new())
    };

    Ok(Json(PlantStateResponse {
        facility_name: state.scenario.facility.name.clone(),
        modbus_sessions: state.sessions.active(),
        active_tags,
        memory,
        coils_summary,
        registers_summary,
    }))
}

pub async fn inject_fault_handler(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<FaultInjectionRequest>,
) -> Result<Json<FaultInjectionResponse>, ApiError> {
    let tag = state
        .scenario
        .tag(&payload.tag_id)
        .ok_or_else(|| ApiError::UnknownTag(payload.tag_id.clone()))?;

    if !tag.area.is_writable() {
        return Err(ApiError::ReadOnlyTag {
            tag_id: tag.id.clone(),
            area: tag.area.to_string(),
        });
    }

    // Register the manual override in the shared registry so simulation ticks do not overwrite it
    {
        let mut ovr_guard = state.overrides.write().await;
        if let Some(duration) = payload.duration_secs {
            let expires_at = state.start_time.elapsed().as_secs_f64() + duration;
            ovr_guard.set_override(tag.id.clone(), payload.forced_value, Some(expires_at));
        } else {
            ovr_guard.set_override(tag.id.clone(), payload.forced_value, None);
        }
    }

    {
        let mut mem = state.shared_memory.write().await;
        encode_tag(&mut mem, tag, payload.forced_value).map_err(|e| ApiError::TagUnwritable {
            tag_id: tag.id.clone(),
            reason: e.to_string(),
        })?;
    }

    Ok(Json(FaultInjectionResponse {
        success: true,
        message: format!(
            "Tag '{}' forced to value {}",
            payload.tag_id, payload.forced_value
        ),
    }))
}

pub async fn clear_fault_handler(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(tag_id): axum::extract::Path<String>,
) -> Result<Json<FaultInjectionResponse>, ApiError> {
    let tag = state
        .scenario
        .tag(&tag_id)
        .ok_or_else(|| ApiError::UnknownTag(tag_id.clone()))?;

    let removed = {
        let mut ovr_guard = state.overrides.write().await;
        ovr_guard.remove_override(&tag.id).is_some()
    };

    if removed {
        Ok(Json(FaultInjectionResponse {
            success: true,
            message: format!("Override cleared for tag '{}'", tag.id),
        }))
    } else {
        Ok(Json(FaultInjectionResponse {
            success: true,
            message: format!("No active override on tag '{}'", tag.id),
        }))
    }
}

/// The ways an API call can fail, each with its own status code.
#[derive(Debug)]
pub enum ApiError {
    UnknownTag(String),
    ReadOnlyTag { tag_id: String, area: String },
    TagUnreadable { tag_id: String, reason: String },
    TagUnwritable { tag_id: String, reason: String },
    HistorianDisabled,
    DatabaseError(String),
    Unauthorized,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        let (status, message) = match &self {
            ApiError::UnknownTag(tag_id) => {
                (StatusCode::NOT_FOUND, format!("Tag '{tag_id}' not found"))
            }
            ApiError::ReadOnlyTag { tag_id, area } => (
                StatusCode::BAD_REQUEST,
                format!("Cannot force tag '{tag_id}': area '{area}' is read-only"),
            ),
            ApiError::TagUnreadable { tag_id, reason } => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Cannot read tag '{tag_id}': {reason}"),
            ),
            ApiError::TagUnwritable { tag_id, reason } => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Cannot write tag '{tag_id}': {reason}"),
            ),
            ApiError::HistorianDisabled => (
                StatusCode::SERVICE_UNAVAILABLE,
                "Historian is not enabled for this facility".to_string(),
            ),
            ApiError::DatabaseError(reason) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Historian database error: {reason}"),
            ),
            ApiError::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                "Unauthorized: missing or invalid API key".to_string(),
            ),
        };

        (
            status,
            Json(FaultInjectionResponse {
                success: false,
                message,
            }),
        )
            .into_response()
    }
}

pub async fn get_alarms_handler(State(state): State<Arc<AppState>>) -> Json<Vec<AlarmResponse>> {
    let snapshot = state.alarms.get_snapshot().await;
    let responses = snapshot
        .into_iter()
        .map(|a| {
            let shelved_remaining_secs = a.shelved_remaining_secs();
            AlarmResponse {
                id: a.config.id,
                description: a.config.description,
                source_tag: a.config.source_tag,
                alarm_type: a.config.alarm_type,
                severity: a.config.severity,
                state: a.state,
                state_code: a.state as u16,
                limit: a.config.limit,
                deadband: a.config.deadband,
                condition_active: a.condition_active,
                current_value: a.current_value,
                last_trip_timestamp_secs: a.last_trip_timestamp_secs,
                shelved_remaining_secs,
            }
        })
        .collect();
    Json(responses)
}

pub async fn acknowledge_alarm_handler(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<Json<AlarmActionResponse>, (StatusCode, Json<AlarmActionResponse>)> {
    match state.alarms.acknowledge(&id).await {
        Ok(new_state) => Ok(Json(AlarmActionResponse {
            success: true,
            alarm_id: id.clone(),
            state: new_state,
            message: format!("Alarm '{id}' acknowledged"),
        })),
        Err(e) => Err((
            StatusCode::NOT_FOUND,
            Json(AlarmActionResponse {
                success: false,
                alarm_id: id,
                state: wireplant_engine::alarm::IsaAlarmState::Normal,
                message: e.to_string(),
            }),
        )),
    }
}

pub async fn reset_alarm_handler(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<Json<AlarmActionResponse>, (StatusCode, Json<AlarmActionResponse>)> {
    match state.alarms.reset(&id).await {
        Ok(new_state) => Ok(Json(AlarmActionResponse {
            success: true,
            alarm_id: id.clone(),
            state: new_state,
            message: format!("Alarm '{id}' reset"),
        })),
        Err(e) => {
            let status = match e {
                wireplant_engine::alarm::AlarmError::NotFound(_) => StatusCode::NOT_FOUND,
                wireplant_engine::alarm::AlarmError::InvalidTransition(_) => {
                    StatusCode::BAD_REQUEST
                }
            };
            Err((
                status,
                Json(AlarmActionResponse {
                    success: false,
                    alarm_id: id,
                    state: wireplant_engine::alarm::IsaAlarmState::Latched,
                    message: e.to_string(),
                }),
            ))
        }
    }
}

pub async fn shelve_alarm_handler(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(id): axum::extract::Path<String>,
    Json(payload): Json<ShelveAlarmRequest>,
) -> Result<Json<AlarmActionResponse>, (StatusCode, Json<AlarmActionResponse>)> {
    if !payload.duration_secs.is_finite()
        || payload.duration_secs <= 0.0
        || payload.duration_secs > (86400.0 * 365.0)
    {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(AlarmActionResponse {
                success: false,
                alarm_id: id,
                state: wireplant_engine::alarm::IsaAlarmState::Normal,
                message:
                    "Duration must be a finite, positive number not exceeding 31536000 seconds"
                        .into(),
            }),
        ));
    }

    let dur = match Duration::try_from_secs_f64(payload.duration_secs) {
        Ok(d) => d,
        Err(_) => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(AlarmActionResponse {
                    success: false,
                    alarm_id: id,
                    state: wireplant_engine::alarm::IsaAlarmState::Normal,
                    message: "Duration in seconds overflows supported range".into(),
                }),
            ));
        }
    };
    match state.alarms.shelve(&id, dur).await {
        Ok(new_state) => Ok(Json(AlarmActionResponse {
            success: true,
            alarm_id: id.clone(),
            state: new_state,
            message: format!("Alarm '{id}' shelved for {}s", payload.duration_secs),
        })),
        Err(e) => Err((
            StatusCode::NOT_FOUND,
            Json(AlarmActionResponse {
                success: false,
                alarm_id: id,
                state: wireplant_engine::alarm::IsaAlarmState::Normal,
                message: e.to_string(),
            }),
        )),
    }
}

pub async fn unshelve_alarm_handler(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<Json<AlarmActionResponse>, (StatusCode, Json<AlarmActionResponse>)> {
    match state.alarms.unshelve(&id).await {
        Ok(new_state) => Ok(Json(AlarmActionResponse {
            success: true,
            alarm_id: id.clone(),
            state: new_state,
            message: format!("Alarm '{id}' unshelved"),
        })),
        Err(e) => Err((
            StatusCode::NOT_FOUND,
            Json(AlarmActionResponse {
                success: false,
                alarm_id: id,
                state: wireplant_engine::alarm::IsaAlarmState::Normal,
                message: e.to_string(),
            }),
        )),
    }
}

pub async fn query_history_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<HistoryQuery>,
) -> Result<Json<HistoryResponse>, ApiError> {
    let Some(historian) = &state.historian else {
        return Err(ApiError::HistorianDisabled);
    };

    let start_ms = query.start_ms.unwrap_or(0);
    let end_ms = query.end_ms.unwrap_or(i64::MAX);
    let limit = query.limit.unwrap_or(1000).min(10000);

    let raw_points = historian
        .query_tag(&query.tag, start_ms, end_ms, limit)
        .await
        .map_err(|e| ApiError::DatabaseError(e.to_string()))?;

    let points = raw_points
        .into_iter()
        .map(|(timestamp_ms, value)| HistoryResponsePoint {
            timestamp_ms,
            value,
        })
        .collect();

    Ok(Json(HistoryResponse {
        tag_id: query.tag,
        points,
    }))
}

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use std::sync::Arc;
use std::time::Instant;
use tower::ServiceExt;
use wireplant_api::{
    ApiServer, AppState, FaultInjectionRequest, FaultInjectionResponse, PlantStateResponse,
};
use wireplant_config::defaults::get_default_scenario_toml;
use wireplant_config::schema::ScenarioConfig;
use wireplant_core::endian::{float32_to_registers, registers_to_float32};
use wireplant_core::memory::{PlantMemory, SessionCounter};
use wireplant_core::overrides::create_shared_overrides;
use wireplant_core::types::MemoryArea;
use wireplant_engine::alarm::{AlarmConfig, AlarmManager, AlarmSeverity, AlarmType, IsaAlarmState};

/// POSTs a fault-injection body and returns the status plus decoded response.
///
/// Every fault endpoint assertion boils down to these two values, so the
/// request plumbing lives here rather than being spelled out per case.
async fn post_fault(
    state: Arc<AppState>,
    tag_id: &str,
    forced_value: f64,
) -> (StatusCode, FaultInjectionResponse) {
    let payload = FaultInjectionRequest {
        tag_id: tag_id.to_string(),
        forced_value,
        duration_secs: None,
    };

    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/fault")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_vec(&payload).unwrap()))
        .unwrap();

    let resp = ApiServer::router(state).oneshot(req).await.unwrap();
    let status = resp.status();
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let decoded = serde_json::from_slice(&body).unwrap();
    (status, decoded)
}

#[tokio::test]
async fn the_health_endpoint_reports_ok() {
    let toml_str = get_default_scenario_toml();
    let scenario: ScenarioConfig = toml::from_str(toml_str).unwrap();
    let facility_name = scenario.facility.name.clone();
    let memory = Arc::new(tokio::sync::RwLock::new(PlantMemory::new(10, 10, 10, 100)));

    let state = Arc::new(AppState {
        scenario: scenario.clone(),
        shared_memory: memory,
        sessions: SessionCounter::default(),
        alarms: AlarmManager::default(),
        historian: None,
        engine_liveness: None,
        overrides: create_shared_overrides(),
        start_time: Instant::now(),
    });

    let app = ApiServer::router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body_bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let health: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(health["status"], "ok");
    assert_eq!(health["facility_name"], facility_name);
    assert!(health["uptime_secs"].as_u64().is_some());
}

#[tokio::test]
async fn the_state_endpoint_reports_every_declared_tag() {
    let toml_str = get_default_scenario_toml();
    let scenario: ScenarioConfig = toml::from_str(toml_str).unwrap();
    let memory = Arc::new(tokio::sync::RwLock::new(PlantMemory::new(
        100, 100, 100, 200,
    )));

    // Populate holding registers for tags in scenario
    {
        let mut mem = memory.write().await;
        // Find holding register tag in scenario
        for tag in &scenario.tags {
            if tag.area == MemoryArea::HoldingRegister {
                let (r0, r1) = float32_to_registers(73.5, tag.endianness);
                mem.write_registers_slice(
                    MemoryArea::HoldingRegister,
                    tag.address as usize,
                    &[r0, r1],
                )
                .unwrap();
            } else if tag.area == MemoryArea::Coil {
                mem.write_bit(MemoryArea::Coil, tag.address as usize, true)
                    .unwrap();
            }
        }
    }

    let state = Arc::new(AppState {
        scenario: scenario.clone(),
        shared_memory: memory,
        sessions: SessionCounter::default(),
        alarms: AlarmManager::default(),
        historian: None,
        engine_liveness: None,
        overrides: create_shared_overrides(),
        start_time: Instant::now(),
    });

    let app = ApiServer::router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/state")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body_bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let state_resp: PlantStateResponse = serde_json::from_slice(&body_bytes).unwrap();

    assert_eq!(state_resp.facility_name, scenario.facility.name);
    assert!(!state_resp.active_tags.is_empty());
    assert_eq!(state_resp.memory.coils_total, 100);
    assert_eq!(state_resp.memory.holding_registers_total, 200);
    assert!(state_resp.coils_summary.is_empty());
    assert!(state_resp.registers_summary.is_empty());

    for tag in &scenario.tags {
        if tag.area == MemoryArea::HoldingRegister {
            let val = state_resp.active_tags.get(&tag.id).copied().unwrap();
            assert!((val - 73.5).abs() < 0.001);
        } else if tag.area == MemoryArea::Coil {
            let val = state_resp.active_tags.get(&tag.id).copied().unwrap();
            assert_eq!(val, 1.0);
        }
    }
}

#[tokio::test]
async fn injecting_a_fault_writes_the_value_and_404s_an_unknown_tag() {
    let toml_str = get_default_scenario_toml();
    let scenario: ScenarioConfig = toml::from_str(toml_str).unwrap();
    let memory = Arc::new(tokio::sync::RwLock::new(PlantMemory::new(
        100, 100, 100, 200,
    )));

    let state = Arc::new(AppState {
        scenario: scenario.clone(),
        shared_memory: memory.clone(),
        sessions: SessionCounter::default(),
        alarms: AlarmManager::default(),
        historian: None,
        engine_liveness: None,
        overrides: create_shared_overrides(),
        start_time: Instant::now(),
    });

    let coil_tag = scenario
        .tags
        .iter()
        .find(|t| t.area == MemoryArea::Coil)
        .expect("should have at least one coil tag");

    let (status, fault_resp) = post_fault(state.clone(), &coil_tag.id, 1.0).await;
    assert_eq!(status, StatusCode::OK);
    assert!(fault_resp.success);

    // The bit should be set in the Process Image, not just echoed back
    {
        let mem = memory.read().await;
        let bit = mem
            .read_bit(MemoryArea::Coil, coil_tag.address as usize)
            .unwrap();
        assert!(bit);
    }

    let hr_tag = scenario
        .tags
        .iter()
        .find(|t| t.area == MemoryArea::HoldingRegister)
        .expect("should have at least one holding register tag");

    let (status, fault_resp) = post_fault(state.clone(), &hr_tag.id, 88.75).await;
    assert_eq!(status, StatusCode::OK);
    assert!(fault_resp.success);

    // 88.75 is not exactly representable as an f32, and it has to survive the
    // endianness round-trip too, so compare with a tolerance rather than `==`.
    {
        let mem = memory.read().await;
        let words = mem
            .read_registers_slice(MemoryArea::HoldingRegister, hr_tag.address as usize, 2)
            .unwrap();
        let decoded = registers_to_float32(words[0], words[1], hr_tag.endianness);
        assert!((decoded - 88.75).abs() < 0.001, "decoded {decoded}");
    }

    // An unknown tag is a 404, and the body still reports failure rather than
    // just relying on the status code.
    let (status, fault_resp) = post_fault(state, "nonexistent_tag_xyz", 99.0).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(!fault_resp.success);
    assert!(fault_resp.message.contains("not found"));
}

#[tokio::test]
async fn the_server_binds_its_socket_answers_health_and_shuts_down() {
    let toml_str = get_default_scenario_toml();
    let scenario: ScenarioConfig = toml::from_str(toml_str).unwrap();
    let memory = Arc::new(tokio::sync::RwLock::new(PlantMemory::new(10, 10, 10, 100)));

    // Bind up front so the address is real before the serving task starts; the
    // old bind-then-drop dance raced the connect below.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let local_addr = listener.local_addr().unwrap().to_string();

    let (shutdown_tx, shutdown_rx) = tokio::sync::broadcast::channel(1);
    let server = ApiServer::new(scenario, memory, SessionCounter::default());

    let handle = tokio::spawn(async move { server.serve(listener, shutdown_rx).await });

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = tokio::net::TcpStream::connect(&local_addr)
        .await
        .expect("connect to the address the server was handed");
    stream
        .write_all(b"GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();

    let mut response = Vec::new();
    tokio::time::timeout(
        std::time::Duration::from_secs(3),
        stream.read_to_end(&mut response),
    )
    .await
    .expect("server did not answer /health in time")
    .unwrap();
    drop(stream);

    let response = String::from_utf8_lossy(&response);
    let status_line = response.lines().next().unwrap_or_default();
    assert_eq!(
        status_line, "HTTP/1.1 200 OK",
        "unexpected response: {status_line}"
    );

    shutdown_tx.send(()).unwrap();

    // Verify server terminates cleanly
    let result = tokio::time::timeout(std::time::Duration::from_secs(3), handle).await;
    assert!(result.is_ok(), "Server timed out shutting down");
    let inner_result = result.unwrap().unwrap();
    assert!(
        inner_result.is_ok(),
        "Server returned error on shutdown: {:?}",
        inner_result
    );
}

/// One state fixture for the malformed-input cases below.
///
/// Sized by the config crate's own rule: a hand-picked table length is how this
/// fixture ends up too small for the addresses the scenario declares.
async fn state_router() -> (axum::Router, Arc<AppState>) {
    let scenario: ScenarioConfig = toml::from_str(get_default_scenario_toml()).unwrap();
    let memory = Arc::new(tokio::sync::RwLock::new(
        wireplant_config::process_image_sizes(&scenario).allocate(),
    ));
    let state = Arc::new(AppState {
        scenario,
        shared_memory: memory,
        sessions: SessionCounter::default(),
        alarms: AlarmManager::default(),
        historian: None,
        engine_liveness: None,
        overrides: create_shared_overrides(),
        start_time: Instant::now(),
    });
    (ApiServer::router(state.clone()), state)
}

async fn post_raw(state: Arc<AppState>, content_type: &str, body: &str) -> StatusCode {
    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/fault")
        .header(header::CONTENT_TYPE, content_type)
        .body(Body::from(body.to_string()))
        .unwrap();

    ApiServer::router(state)
        .oneshot(request)
        .await
        .unwrap()
        .status()
}

#[tokio::test]
async fn a_fault_body_that_is_not_json_is_rejected_rather_than_guessed_at() {
    let (_, state) = state_router().await;

    let status = post_raw(state.clone(), "application/json", "{not json").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let status = post_raw(state.clone(), "text/plain", "tag_id=x").await;
    assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE);

    let status = post_raw(state, "application/json", "").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn a_fault_that_names_a_tag_outside_the_process_image_is_a_server_error() {
    let scenario: ScenarioConfig = toml::from_str(get_default_scenario_toml()).unwrap();
    // Deliberately too small for the scenario's highest address, which is the
    // sizing mistake the old `unwrap_or(0)` used to hide.
    let memory = Arc::new(tokio::sync::RwLock::new(PlantMemory::new(2, 2, 2, 2)));
    let state = Arc::new(AppState {
        scenario,
        shared_memory: memory,
        sessions: SessionCounter::default(),
        alarms: AlarmManager::default(),
        historian: None,
        engine_liveness: None,
        overrides: create_shared_overrides(),
        start_time: Instant::now(),
    });

    let (status, body) = post_fault(state, "penstock_pressure_psi", 99.0).await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(!body.success);
    assert!(
        body.message.contains("penstock_pressure_psi"),
        "the body should name the tag: {}",
        body.message
    );
}

#[tokio::test]
async fn the_state_response_reports_the_session_count_the_listener_keeps() {
    let scenario: ScenarioConfig = toml::from_str(get_default_scenario_toml()).unwrap();
    let memory = Arc::new(tokio::sync::RwLock::new(
        wireplant_config::process_image_sizes(&scenario).allocate(),
    ));
    let sessions = SessionCounter::default();
    // Two sessions the endpoint must report; the guards are held for the rest
    // of the test so the count does not drop back to zero.
    let _first = sessions.try_open(10).expect("room for a first session");
    let _second = sessions.try_open(10).expect("room for a second session");

    let state = Arc::new(AppState {
        scenario,
        shared_memory: memory,
        sessions,
        alarms: AlarmManager::default(),
        historian: None,
        engine_liveness: None,
        overrides: create_shared_overrides(),
        start_time: Instant::now(),
    });

    let response = ApiServer::router(state)
        .oneshot(
            Request::builder()
                .uri("/api/v1/state")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let decoded: PlantStateResponse = serde_json::from_slice(&body).unwrap();

    assert_eq!(decoded.modbus_sessions, 2);
}

#[tokio::test]
async fn state_endpoint_returns_raw_memory_when_requested() {
    let toml_str = get_default_scenario_toml();
    let scenario: ScenarioConfig = toml::from_str(toml_str).unwrap();
    let memory = Arc::new(tokio::sync::RwLock::new(PlantMemory::new(
        100, 100, 100, 200,
    )));

    let state = Arc::new(AppState {
        scenario,
        shared_memory: memory,
        sessions: SessionCounter::default(),
        alarms: AlarmManager::default(),
        historian: None,
        engine_liveness: None,
        overrides: create_shared_overrides(),
        start_time: Instant::now(),
    });

    let response = ApiServer::router(state)
        .oneshot(
            Request::builder()
                .uri("/api/v1/state?include_raw=true")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let decoded: PlantStateResponse = serde_json::from_slice(&body).unwrap();

    assert_eq!(decoded.memory.coils_total, 100);
    assert_eq!(decoded.memory.holding_registers_total, 200);
    assert_eq!(decoded.coils_summary.len(), 100);
    assert_eq!(decoded.registers_summary.len(), 200);
}

#[tokio::test]
async fn alarm_rest_endpoints_lifecycle() {
    let toml_str = get_default_scenario_toml();
    let mut scenario: ScenarioConfig = toml::from_str(toml_str).unwrap();
    let alarm_cfg = AlarmConfig {
        id: "penstock_hi".into(),
        description: "Penstock High Pressure".into(),
        source_tag: "penstock_pressure_psi".into(),
        alarm_type: AlarmType::High,
        limit: 150.0,
        deadband: 2.0,
        severity: AlarmSeverity::Critical,
        on_delay_secs: 0.0,
        off_delay_secs: 0.0,
        latching: true,
        max_shelve_duration_secs: 3600.0,
        active_coil_addr: Some(10),
        unack_coil_addr: Some(11),
        ack_coil_addr: Some(12),
        reset_coil_addr: Some(13),
        state_reg_addr: Some(50),
    };
    scenario.alarms = vec![alarm_cfg];

    let memory = Arc::new(tokio::sync::RwLock::new(
        wireplant_config::process_image_sizes(&scenario).allocate(),
    ));
    let alarms = AlarmManager::from_configs(&scenario.alarms);

    // Trip alarm
    let mut values = std::collections::HashMap::new();
    values.insert("penstock_pressure_psi".into(), 160.0);
    alarms
        .step(&values, std::time::Duration::from_millis(100), 0.1)
        .await;

    let state = Arc::new(AppState {
        scenario,
        shared_memory: memory,
        sessions: SessionCounter::default(),
        alarms,
        historian: None,
        engine_liveness: None,
        overrides: create_shared_overrides(),
        start_time: Instant::now(),
    });

    let app = ApiServer::router(state.clone());

    // 1. GET /api/v1/alarms -> should report Unacknowledged
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/alarms")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let list: Vec<wireplant_api::AlarmResponse> = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].state, IsaAlarmState::Unacknowledged);
    assert!(list[0].condition_active);

    // 2. POST /api/v1/alarms/penstock_hi/acknowledge -> Acknowledged
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/alarms/penstock_hi/acknowledge")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let action_resp: wireplant_api::AlarmActionResponse = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(action_resp.state, IsaAlarmState::Acknowledged);

    // 3. POST /api/v1/alarms/penstock_hi/shelve -> Shelved
    let shelve_payload = wireplant_api::ShelveAlarmRequest {
        duration_secs: 300.0,
    };
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/alarms/penstock_hi/shelve")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&shelve_payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // 4. POST /api/v1/alarms/penstock_hi/unshelve -> Unacknowledged (since 160.0 >= 150.0)
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/alarms/penstock_hi/unshelve")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let action_resp: wireplant_api::AlarmActionResponse = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(action_resp.state, IsaAlarmState::Unacknowledged);

    // Ack again, then return process to normal -> becomes Latched (latching = true)
    state.alarms.acknowledge("penstock_hi").await.unwrap();
    values.insert("penstock_pressure_psi".into(), 100.0);
    state
        .alarms
        .step(&values, std::time::Duration::from_millis(100), 0.2)
        .await;

    let snapshot = state.alarms.get_snapshot().await;
    assert_eq!(snapshot[0].state, IsaAlarmState::Latched);

    // 5. POST /api/v1/alarms/penstock_hi/reset -> Normal
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/alarms/penstock_hi/reset")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let action_resp: wireplant_api::AlarmActionResponse = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(action_resp.state, IsaAlarmState::Normal);
}

#[tokio::test]
async fn historian_query_endpoint_returns_data() {
    let toml_str = get_default_scenario_toml();
    let scenario: ScenarioConfig = toml::from_str(toml_str).unwrap();
    let memory = Arc::new(tokio::sync::RwLock::new(
        wireplant_config::process_image_sizes(&scenario).allocate(),
    ));

    let hist_cfg = wireplant_engine::HistorianConfig {
        db_path: std::path::PathBuf::from(":memory:"),
        flush_interval: std::time::Duration::from_millis(100),
        batch_size: 10,
        retention_hours: 24,
    };

    let tags: Vec<wireplant_engine::HistorianTagMeta> = scenario
        .tags
        .iter()
        .map(|t| wireplant_engine::HistorianTagMeta {
            id: t.id.clone(),
            description: t.description.clone().unwrap_or_default(),
            units: t.units.clone(),
            data_type: format!("{:?}", t.data_type),
        })
        .collect();

    let historian = Arc::new(wireplant_engine::Historian::new(hist_cfg, &tags).unwrap());

    // Insert historical points
    let samples = vec![
        (1000, "penstock_pressure_psi", 120.0),
        (2000, "penstock_pressure_psi", 125.5),
        (3000, "penstock_pressure_psi", 130.0),
    ];
    historian.insert_samples(&samples).await.unwrap();

    let state = Arc::new(AppState {
        scenario,
        shared_memory: memory,
        sessions: SessionCounter::default(),
        alarms: AlarmManager::default(),
        historian: Some(historian),
        engine_liveness: None,
        overrides: create_shared_overrides(),
        start_time: Instant::now(),
    });

    let app = ApiServer::router(state.clone());

    // Query 1500..2500 -> should return only point at 2000ms
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/history?tag=penstock_pressure_psi&start_ms=1500&end_ms=2500")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let history: wireplant_api::HistoryResponse = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(history.tag_id, "penstock_pressure_psi");
    assert_eq!(history.points.len(), 1);
    assert_eq!(history.points[0].timestamp_ms, 2000);
    assert_eq!(history.points[0].value, 125.5);
}

#[tokio::test]
async fn prometheus_metrics_endpoint_returns_valid_exposition() {
    let toml_str = get_default_scenario_toml();
    let scenario: ScenarioConfig = toml::from_str(toml_str).unwrap();
    let memory = Arc::new(tokio::sync::RwLock::new(
        wireplant_config::process_image_sizes(&scenario).allocate(),
    ));

    let state = Arc::new(AppState {
        scenario,
        shared_memory: memory,
        sessions: SessionCounter::default(),
        alarms: AlarmManager::default(),
        historian: None,
        engine_liveness: None,
        overrides: create_shared_overrides(),
        start_time: Instant::now(),
    });

    let resp = ApiServer::router(state)
        .oneshot(
            Request::builder()
                .uri("/metrics")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let content_type = resp
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    assert!(content_type.contains("text/plain"));

    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let text = String::from_utf8(bytes.to_vec()).unwrap();

    assert!(text.contains("wireplant_uptime_seconds"));
    assert!(text.contains("wireplant_modbus_sessions"));
    assert!(text.contains("wireplant_process_tag{tag_id=\"penstock_pressure_psi\""));

    for line in text.lines() {
        if line.is_empty() {
            continue;
        }
        if line.starts_with("# HELP") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            assert!(parts.len() >= 3, "Invalid # HELP directive: {line}");
        } else if line.starts_with("# TYPE") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            assert_eq!(parts.len(), 4, "Invalid # TYPE directive: {line}");
            assert!(matches!(
                parts[3],
                "gauge" | "counter" | "histogram" | "summary"
            ));
        } else {
            // Metric sample line: metric_name{...} value
            let mut parts = line.split_whitespace();
            let _metric_and_labels = parts.next().expect("metric identifier");
            let val_str = parts.next().expect("metric value");
            assert!(
                parts.next().is_none(),
                "Unexpected trailing fields in line: {line}"
            );

            let is_valid_float = val_str == "+Inf"
                || val_str == "-Inf"
                || val_str == "NaN"
                || val_str.parse::<f64>().is_ok();
            assert!(is_valid_float, "Invalid Prometheus float: '{val_str}'");
        }
    }
}

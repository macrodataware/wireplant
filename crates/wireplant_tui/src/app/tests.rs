use super::*;
use std::sync::Arc;
use tokio::sync::RwLock;
use wireplant_config::schema::{
    FacilityConfig, FaultAction, FaultProfile, NetworkConfig, TagConfig,
};
use wireplant_core::endian::float32_to_registers;
use wireplant_core::memory::PlantMemory;
use wireplant_core::overrides::create_shared_overrides;
use wireplant_core::types::{Endianness, TagDataType};
use wireplant_engine::alarm::{AlarmConfig, AlarmSeverity, IsaAlarmState};
use wireplant_engine::waveforms::WaveformSpec;

fn pressure_tag() -> TagConfig {
    TagConfig {
        id: "main_pressure_psi".to_string(),
        description: Some("Pressure sensor".to_string()),
        area: MemoryArea::HoldingRegister,
        address: 0,
        data_type: TagDataType::Float32,
        endianness: Endianness::BigEndian,
        units: "PSI".to_string(),
        waveform: WaveformSpec::Constant { value: 100.0 },
    }
}

fn alarm_coil() -> TagConfig {
    TagConfig {
        id: "trip_alarm_coil".to_string(),
        description: Some("Overpressure alarm indicator".to_string()),
        area: MemoryArea::Coil,
        address: 0,
        data_type: TagDataType::Boolean,
        endianness: Endianness::BigEndian,
        units: String::new(),
        waveform: WaveformSpec::Constant { value: 0.0 },
    }
}

fn sample_scenario() -> ScenarioConfig {
    ScenarioConfig {
        facility: FacilityConfig {
            name: "Alpha Test Facility".to_string(),
            description: None,
            tick_rate_ms: 100,
        },
        memory: wireplant_config::schema::MemoryConfig::default(),
        network: NetworkConfig::default(),
        tags: vec![pressure_tag(), alarm_coil()],
        processes: Vec::new(),
        fault_profiles: vec![FaultProfile {
            id: "penstock_burst".to_string(),
            name: "Penstock Overpressure Emergency".to_string(),
            description: None,
            actions: vec![
                FaultAction {
                    target_tag: "main_pressure_psi".to_string(),
                    force_value: 195.0,
                    duration_secs: Some(10.0),
                },
                FaultAction {
                    target_tag: "trip_alarm_coil".to_string(),
                    force_value: 1.0,
                    duration_secs: None,
                },
            ],
        }],
        alarms: vec![AlarmConfig {
            id: "high_pressure".to_string(),
            description: "Overpressure warning".to_string(),
            source_tag: "main_pressure_psi".to_string(),
            alarm_type: wireplant_engine::alarm::AlarmType::High,
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
        }],
        historian: wireplant_config::schema::HistorianConfigSchema::default(),
    }
}

fn app_with(scenario: ScenarioConfig) -> (TuiApp, SharedProcessImage) {
    let memory: SharedProcessImage = Arc::new(RwLock::new(PlantMemory::new(10, 10, 10, 10)));
    let (_tx, rx) = watch::channel(EngineTickEvent {
        tick_number: 0,
        elapsed_secs: 0.0,
        tag_values: HashMap::new(),
    });
    let alarms = AlarmManager::from_configs(&scenario.alarms);
    let sessions = SessionCounter::default();
    (
        TuiApp::new(
            scenario,
            memory.clone(),
            rx,
            alarms,
            sessions,
            None,
            create_shared_overrides(),
            None,
        ),
        memory,
    )
}

fn sample_app() -> (TuiApp, SharedProcessImage) {
    app_with(sample_scenario())
}

#[test]
fn initial_app_state_selects_first_tag() {
    let (app, _) = sample_app();

    assert_eq!(app.active_tab, ActiveTab::ProcessTags);
    assert_eq!(app.mode, AppMode::Normal);
    assert_eq!(app.selected_tag_index, 0);
    assert!(app.active_overrides.is_empty());
    assert_eq!(app.sparkline_data.len(), 2);
    assert!(app.sparkline_data["main_pressure_psi"].is_empty());
    assert!(app.is_running);
}

#[test]
fn tab_cycling_wraps_in_both_directions() {
    let (mut app, _) = sample_app();

    for expected in [
        ActiveTab::Alarms,
        ActiveTab::ProcessControl,
        ActiveTab::NetworkSessions,
        ActiveTab::MemoryMap,
        ActiveTab::FaultStudio,
        ActiveTab::EventLog,
        ActiveTab::ProcessTags,
    ] {
        app.tab_next();
        assert_eq!(app.active_tab, expected);
    }

    app.tab_prev();
    assert_eq!(app.active_tab, ActiveTab::EventLog);
    app.tab_prev();
    assert_eq!(app.active_tab, ActiveTab::FaultStudio);

    app.set_tab(ActiveTab::NetworkSessions);
    assert_eq!(app.active_tab, ActiveTab::NetworkSessions);
}

#[test]
fn tag_selection_clamps_at_both_ends() {
    let (mut app, _) = sample_app();

    app.select_up();
    assert_eq!(app.selected_tag_index, 0);

    app.select_down();
    app.select_down();
    assert_eq!(app.selected_tag_index, 1);

    app.select_up();
    app.select_up();
    assert_eq!(app.selected_tag_index, 0);
}

#[test]
fn committing_the_modal_writes_the_register_and_pins_the_value() {
    let (mut app, memory) = sample_app();

    app.open_override_modal();
    match &app.mode {
        AppMode::ValueInputModal { tag_id, .. } => assert_eq!(tag_id, "main_pressure_psi"),
        other => panic!("expected the value modal, got {other:?}"),
    }

    app.mode = AppMode::ValueInputModal {
        tag_id: "main_pressure_psi".to_string(),
        input_buffer: "142.5".to_string(),
        duration_buffer: String::new(),
        is_discrete: false,
    };
    app.commit_override_modal();

    assert_eq!(app.mode, AppMode::Normal);
    assert_eq!(
        app.active_overrides
            .get("main_pressure_psi")
            .map(|o| o.value),
        Some(142.5)
    );

    let mem = memory.try_read().unwrap();
    let r0 = mem
        .read_registers_slice(MemoryArea::HoldingRegister, 0, 1)
        .unwrap()[0];
    let r1 = mem
        .read_registers_slice(MemoryArea::HoldingRegister, 1, 1)
        .unwrap()[0];
    assert_eq!((r0, r1), float32_to_registers(142.5, Endianness::BigEndian));
}

#[test]
fn modal_rejects_non_numeric_input() {
    let (mut app, memory) = sample_app();

    app.mode = AppMode::ValueInputModal {
        tag_id: "main_pressure_psi".to_string(),
        input_buffer: "--..".to_string(),
        duration_buffer: String::new(),
        is_discrete: false,
    };
    app.commit_override_modal();

    assert_eq!(app.mode, AppMode::Normal);
    assert!(app.active_overrides.is_empty());
    assert_eq!(
        memory.try_read().unwrap().holding_registers[0],
        0,
        "a rejected value must not reach the Process Image"
    );
    assert_eq!(
        app.log_entries.back().map(|e| e.level),
        Some(LogLevel::Error)
    );
}

#[test]
fn toggling_a_discrete_tag_sets_and_clears_the_bit() {
    let (mut app, memory) = sample_app();
    app.selected_tag_index = 1;
    assert_eq!(app.selected_tag().unwrap().id, "trip_alarm_coil");

    app.toggle_discrete();
    assert_eq!(
        app.active_overrides.get("trip_alarm_coil").map(|o| o.value),
        Some(1.0)
    );
    assert!(memory
        .try_read()
        .unwrap()
        .read_bit(MemoryArea::Coil, 0)
        .unwrap());

    app.toggle_discrete();
    assert_eq!(
        app.active_overrides.get("trip_alarm_coil").map(|o| o.value),
        Some(0.0)
    );
    assert!(!memory
        .try_read()
        .unwrap()
        .read_bit(MemoryArea::Coil, 0)
        .unwrap());
}

#[test]
fn toggling_an_analog_tag_does_nothing() {
    let (mut app, memory) = sample_app();
    app.selected_tag_index = 0;

    app.toggle_discrete();

    assert!(app.active_overrides.is_empty());
    assert_eq!(memory.try_read().unwrap().holding_registers[0], 0);
}

#[test]
fn clearing_removes_only_the_selected_override() {
    let (mut app, _) = sample_app();
    app.pin_override("main_pressure_psi", 120.0, None);
    app.pin_override("trip_alarm_coil", 1.0, None);

    app.selected_tag_index = 0;
    app.clear_override_selected();
    assert!(!app.active_overrides.contains_key("main_pressure_psi"));
    assert!(app.active_overrides.contains_key("trip_alarm_coil"));

    app.clear_all_overrides();
    assert!(app.active_overrides.is_empty());
}

#[test]
fn fault_profile_applies_actions_and_durations() {
    let (mut app, memory) = sample_app();

    app.trigger_fault_profile(0);

    assert_eq!(app.active_overrides.len(), 2, "both actions should stick");
    assert!(app.active_overrides["main_pressure_psi"]
        .expires_at_secs
        .is_some());
    assert!(app.active_overrides["trip_alarm_coil"]
        .expires_at_secs
        .is_none());
    assert!(memory
        .try_read()
        .unwrap()
        .read_bit(MemoryArea::Coil, 0)
        .unwrap());
}

#[test]
fn timed_override_expires_on_schedule() {
    let (mut app, _) = sample_app();
    app.pin_override("main_pressure_psi", 999.0, Some(10.0));
    app.pin_override("trip_alarm_coil", 1.0, None);

    app.on_tick(9.9);
    assert_eq!(app.active_overrides.len(), 2);

    app.on_tick(10.0);
    assert!(!app.active_overrides.contains_key("main_pressure_psi"));
    assert!(app.active_overrides.contains_key("trip_alarm_coil"));
}

#[test]
fn the_trend_shows_the_override_and_caps_its_own_length() {
    let (mut app, _) = sample_app();
    let (tx, rx) = watch::channel(EngineTickEvent {
        tick_number: 0,
        elapsed_secs: 0.0,
        tag_values: HashMap::new(),
    });
    app.tick_rx = rx;
    app.pin_override("main_pressure_psi", 7.0, None);

    let mut values = HashMap::new();
    values.insert("main_pressure_psi".to_string(), 100.0);
    tx.send(EngineTickEvent {
        tick_number: 1,
        elapsed_secs: 0.1,
        tag_values: values,
    })
    .unwrap();

    app.on_tick(0.1);
    assert_eq!(app.sparkline_data["main_pressure_psi"].back(), Some(&7.0));

    for _ in 0..SPARKLINE_CAPACITY + 20 {
        app.push_sparkline("main_pressure_psi", 1.0);
    }
    assert_eq!(
        app.sparkline_data["main_pressure_psi"].len(),
        SPARKLINE_CAPACITY,
        "the ring should not grow past its capacity"
    );
}

#[test]
fn the_event_log_ring_does_not_grow_past_its_capacity() {
    let (mut app, _) = sample_app();

    for i in 0..LOG_CAPACITY + 50 {
        app.add_log(LogLevel::Info, format!("line {i}"));
    }

    assert_eq!(app.log_entries.len(), LOG_CAPACITY);
    assert_eq!(
        app.log_entries
            .back()
            .map(|e| e.message.as_str())
            .unwrap_or_default(),
        format!("line {}", LOG_CAPACITY + 49)
    );
}

#[tokio::test]
async fn alarm_lifecycle_acknowledges_and_resets() {
    let (mut app, _) = sample_app();
    app.sync_alarms_from_inner();
    assert_eq!(app.cached_alarms.len(), 1);
    assert_eq!(app.cached_alarms[0].state, IsaAlarmState::Normal);

    // Trip the alarm by exceeding limit (limit = 175.0, trip with 190.0)
    let mut vals = HashMap::new();
    vals.insert("main_pressure_psi".to_string(), 190.0);
    app.alarms
        .step(&vals, std::time::Duration::from_millis(600), 0.6)
        .await;
    app.refresh_alarms().await;
    assert_eq!(app.cached_alarms[0].state, IsaAlarmState::Unacknowledged);

    // Acknowledge alarm
    app.alarm_table_state.select(Some(0));
    app.acknowledge_selected_alarm().await;
    assert_eq!(app.cached_alarms[0].state, IsaAlarmState::Acknowledged);

    // Return PV to normal (140.0 < limit 150.0 - deadband 2.0). Since latching=true, alarm transitions to Latched
    vals.insert("main_pressure_psi".to_string(), 140.0);
    app.alarms
        .step(&vals, std::time::Duration::from_millis(1200), 1.2)
        .await;
    app.refresh_alarms().await;
    assert_eq!(app.cached_alarms[0].state, IsaAlarmState::Latched);

    // Operator resets the latched alarm -> returns to Normal
    app.reset_selected_alarm().await;
    assert_eq!(app.cached_alarms[0].state, IsaAlarmState::Normal);
}

#[tokio::test]
async fn all_seven_tabs_and_modals_render_without_panicking() {
    let (mut app, _) = sample_app();
    let backend = ratatui::backend::TestBackend::new(120, 30);
    let mut terminal = ratatui::Terminal::new(backend).unwrap();

    // 1. Render all 7 tabs
    for tab in ActiveTab::ALL {
        app.set_tab(tab);
        terminal.draw(|f| draw_ui(f, &mut app)).unwrap();
    }

    // 2. Render on constrained 80x24 terminal
    let backend_small = ratatui::backend::TestBackend::new(80, 24);
    let mut terminal_small = ratatui::Terminal::new(backend_small).unwrap();
    for tab in ActiveTab::ALL {
        app.set_tab(tab);
        terminal_small.draw(|f| draw_ui(f, &mut app)).unwrap();
    }

    // 3. Render all modal dialogs
    app.mode = AppMode::ValueInputModal {
        tag_id: "main_pressure_psi".to_string(),
        input_buffer: "150.0".to_string(),
        duration_buffer: "10".to_string(),
        is_discrete: false,
    };
    terminal.draw(|f| draw_ui(f, &mut app)).unwrap();

    app.mode = AppMode::ShelveAlarmModal {
        alarm_id: "high_pressure".to_string(),
        duration_buffer: "300".to_string(),
    };
    terminal.draw(|f| draw_ui(f, &mut app)).unwrap();

    app.mode = AppMode::TagSearchModal {
        query_buffer: "pressure".to_string(),
    };
    terminal.draw(|f| draw_ui(f, &mut app)).unwrap();

    app.mode = AppMode::FaultProfileMenu { selected_index: 0 };
    terminal.draw(|f| draw_ui(f, &mut app)).unwrap();

    app.mode = AppMode::HelpOverlay;
    terminal.draw(|f| draw_ui(f, &mut app)).unwrap();
}

#[tokio::test]
async fn operator_can_acknowledge_shelve_and_unshelve_alarms() {
    let (mut app, _) = sample_app();

    // Trip alarm
    let mut vals = HashMap::new();
    vals.insert("main_pressure_psi".to_string(), 160.0);
    app.alarms
        .step(&vals, std::time::Duration::from_millis(100), 0.1)
        .await;

    app.refresh_alarms().await;
    assert_eq!(app.active_alarms_count, 1);
    assert_eq!(app.unacknowledged_alarms_count, 1);
    assert_eq!(app.cached_alarms[0].state, IsaAlarmState::Unacknowledged);

    // Acknowledge
    app.alarm_table_state.select(Some(0));
    app.acknowledge_selected_alarm().await;
    assert_eq!(app.unacknowledged_alarms_count, 0);
    assert_eq!(app.cached_alarms[0].state, IsaAlarmState::Acknowledged);

    // Shelve
    app.mode = AppMode::ShelveAlarmModal {
        alarm_id: "high_pressure".to_string(),
        duration_buffer: "120".to_string(),
    };
    app.commit_shelve_modal().await;
    assert_eq!(app.cached_alarms[0].state, IsaAlarmState::Shelved);

    // Unshelve
    app.unshelve_selected_alarm().await;
    assert_eq!(app.cached_alarms[0].state, IsaAlarmState::Unacknowledged);
}

#[test]
fn tags_can_be_filtered_by_area_and_searched_by_name() {
    let (mut app, _) = sample_app();

    // No filter: all 2 tags
    assert_eq!(app.filtered_tags().len(), 2);

    // Filter area: HoldingRegister
    app.tag_area_filter = TagAreaFilter::HoldingRegister;
    let hrs = app.filtered_tags();
    assert_eq!(hrs.len(), 1);
    assert_eq!(hrs[0].id, "main_pressure_psi");

    // Filter area: Coil
    app.tag_area_filter = TagAreaFilter::Coil;
    let coils = app.filtered_tags();
    assert_eq!(coils.len(), 1);
    assert_eq!(coils[0].id, "trip_alarm_coil");

    // Substring search
    app.tag_area_filter = TagAreaFilter::All;
    app.tag_search_query = "main_pressure".to_string();
    let matches = app.filtered_tags();
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].id, "main_pressure_psi");

    app.tag_search_query = "nonexistent".to_string();
    assert!(app.filtered_tags().is_empty());
}

#[tokio::test]
async fn real_scenarios_render_cleanly() {
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root_dir = manifest_dir.join("../..");
    let scenario_files = [
        root_dir.join("scenarios/default_hydro.toml"),
        root_dir.join("scenarios/closed_loop_water_plant.toml"),
        root_dir.join("scenarios/water_treatment.toml"),
        root_dir.join("scenarios/petrochemical.toml"),
    ];

    let backend = ratatui::backend::TestBackend::new(120, 30);
    let mut terminal = ratatui::Terminal::new(backend).unwrap();

    for file_path in scenario_files {
        let toml_str = std::fs::read_to_string(&file_path)
            .unwrap_or_else(|e| panic!("Failed to read {}: {e}", file_path.display()));
        let scenario = ScenarioConfig::from_toml(&toml_str).unwrap();
        let memory: SharedProcessImage = Arc::new(RwLock::new(
            wireplant_config::process_image_sizes(&scenario).allocate(),
        ));
        let alarms = AlarmManager::from_configs(&scenario.alarms);
        let (_tx, rx) = watch::channel(EngineTickEvent {
            tick_number: 0,
            elapsed_secs: 0.0,
            tag_values: HashMap::new(),
        });

        let mut app = TuiApp::new(
            scenario,
            memory,
            rx,
            alarms,
            SessionCounter::default(),
            None,
            create_shared_overrides(),
            None,
        );

        for tab in ActiveTab::ALL {
            app.set_tab(tab);
            terminal.draw(|f| draw_ui(f, &mut app)).unwrap();
        }
    }
}

#[tokio::test]
async fn tracing_bridge_delivers_logs_to_tui() {
    use tracing_subscriber::layer::SubscriberExt;

    let (log_tx, log_rx) = mpsc::unbounded_channel();
    let (mut app, _) = sample_app();
    app.log_rx = Some(log_rx);

    let layer = crate::tracing_bridge::TuiTracingLayer::new(log_tx, Instant::now());
    let subscriber = tracing_subscriber::registry().with(layer);

    // Emit an actual tracing event scoped through the subscriber
    tracing::subscriber::with_default(subscriber, || {
        tracing::info!("Telemetry link established with remote PLC");
    });

    app.poll_incoming_logs();
    assert!(
        app.log_entries.iter().any(|e| e
            .message
            .contains("Telemetry link established with remote PLC")),
        "Tracing bridge subscriber failed to deliver message to TUI log entries"
    );
}

#[test]
fn tui_override_updates_shared_overrides_registry() {
    let (mut app, _) = sample_app();
    assert!(!app
        .overrides
        .try_read()
        .unwrap()
        .is_overridden("main_pressure_psi"));

    app.inject_timed_override("main_pressure_psi", 75.0, Some(60.0));
    assert_eq!(
        app.overrides
            .try_read()
            .unwrap()
            .get_value("main_pressure_psi"),
        Some(75.0)
    );

    app.clear_override_selected();
    assert!(!app
        .overrides
        .try_read()
        .unwrap()
        .is_overridden("main_pressure_psi"));
}

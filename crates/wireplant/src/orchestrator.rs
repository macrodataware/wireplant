use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::{broadcast, RwLock};
use tracing::{error, info};
use wireplant_api::ApiServer;
use wireplant_config::defaults::get_default_scenario_toml;
use wireplant_config::schema::ScenarioConfig;
use wireplant_config::validation::validate_scenario;
use wireplant_core::memory::SessionCounter;
use wireplant_core::memory::SharedProcessImage;
use wireplant_core::overrides::{create_shared_overrides, SharedOverrides};
use wireplant_engine::process::ActiveProcess;
use wireplant_engine::ticker::{EngineTickEvent, SimulationEngine, TagBinding};
use wireplant_engine::waveforms::ActiveWaveform;
use wireplant_modbus::ModbusServer;
use wireplant_mqtt::MqttPublisher;
use wireplant_opcua::OpcUaServer;
use wireplant_tui::TuiApp;

use crate::args::CliArgs;
use crate::shutdown::wait_for_shutdown_signal;

/// Boots the whole simulation and owns the lifetime of every task in it.
///
/// There is no struct state: `run` builds the Process Image, binds the
/// listeners, spawns the engine and the protocol servers, then blocks on either
/// the TUI loop or a shutdown signal. Adding a protocol means adding one bind,
/// one spawn block and one `shutdown_tx.subscribe()`.
pub(crate) struct Orchestrator;

impl Orchestrator {
    pub(crate) async fn run(
        args: CliArgs,
        log_rx: Option<tokio::sync::mpsc::UnboundedReceiver<wireplant_tui::LogEntry>>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let scenario_toml = match &args.scenario {
            Some(path) => std::fs::read_to_string(path)?,
            None => get_default_scenario_toml().to_string(),
        };
        let mut scenario = ScenarioConfig::from_toml(&scenario_toml)?;

        // Command-line flags win over the file, so they are folded in before
        // the second validation pass: a bad `--modbus-bind` is reported by the
        // same validator that would have caught it in the TOML.
        apply_flag_overrides(&mut scenario, &args);
        validate_scenario(&scenario)?;

        info!("Starting WirePlant: {}", scenario.facility.name);

        let shared_memory = build_process_image(&scenario);
        let shared_overrides = create_shared_overrides();
        let (shutdown_tx, _) = broadcast::channel::<()>(1);
        let engine_liveness = Arc::new(AtomicBool::new(false));
        let (tick_rx, alarms, mut engine_handle) = spawn_engine(
            &scenario,
            &shared_memory,
            shared_overrides.clone(),
            &shutdown_tx,
            engine_liveness.clone(),
        );

        // Bind before spawning anything: a port already in use is a startup
        // failure the operator should see, not a task that dies quietly while
        // the rest of the plant comes up. A disabled protocol is not bound at
        // all, so its address cannot fail the boot.
        let sessions = SessionCounter::default();
        if scenario.network.modbus.enabled {
            let listener = bind(&scenario.network.modbus.bind_addr).await?;
            let server = ModbusServer::new(
                scenario.network.modbus.default_unit_id,
                scenario.network.modbus.allowed_unit_ids.clone(),
                scenario.network.modbus.max_connections,
                sessions.clone(),
                shared_memory.clone(),
            );
            let cancel = shutdown_tx.subscribe();
            tokio::spawn(async move {
                server.serve(listener, cancel).await;
            });
        } else {
            info!("Modbus TCP disabled by the scenario");
        }

        if scenario.network.modbus.serial_enabled {
            if let Some(port) = &scenario.network.modbus.serial_port {
                let rtu_server = wireplant_modbus::ModbusRtuServer::new(
                    port.clone(),
                    scenario.network.modbus.baud_rate,
                    scenario.network.modbus.parity.clone(),
                    scenario.network.modbus.data_bits,
                    scenario.network.modbus.stop_bits,
                    scenario.network.modbus.allowed_unit_ids.clone(),
                    shared_memory.clone(),
                );
                let cancel = shutdown_tx.subscribe();
                tokio::spawn(async move {
                    rtu_server.run(cancel).await;
                });
            }
        }

        let historian = if scenario.historian.enabled {
            let hist_cfg = wireplant_engine::HistorianConfig {
                db_path: std::path::PathBuf::from(&scenario.historian.db_path),
                flush_interval: std::time::Duration::from_millis(
                    scenario.historian.flush_interval_ms,
                ),
                batch_size: scenario.historian.batch_size,
                retention_hours: scenario.historian.retention_hours,
            };
            let tags: Vec<wireplant_engine::HistorianTagMeta> = scenario
                .tags
                .iter()
                .map(|t| wireplant_engine::HistorianTagMeta {
                    id: t.id.clone(),
                    description: t.description.clone().unwrap_or_default(),
                    units: t.units.clone(),
                    data_type: t.data_type.as_str().to_string(),
                })
                .collect();
            match wireplant_engine::Historian::new(hist_cfg, &tags) {
                Ok(h) => {
                    let h_arc = Arc::new(h);
                    let ingest_hist = h_arc.clone();
                    let ingest_rx = tick_rx.clone();
                    let ingest_cancel = shutdown_tx.subscribe();
                    tokio::spawn(async move {
                        ingest_hist
                            .run_ingestion_loop(ingest_rx, ingest_cancel)
                            .await;
                    });

                    let pruner_hist = h_arc.clone();
                    let pruner_cancel = shutdown_tx.subscribe();
                    tokio::spawn(async move {
                        pruner_hist.run_retention_pruner(pruner_cancel).await;
                    });

                    // Forward alarm lifecycle transitions to the SQLite historian worker
                    let mut alarm_events_rx = alarms.subscribe_transitions();
                    let hist_worker = h_arc.clone();
                    let alarm_cancel = shutdown_tx.subscribe();
                    tokio::spawn(async move {
                        let mut cancel = alarm_cancel;
                        loop {
                            tokio::select! {
                                _ = cancel.recv() => break,
                                Ok(event) = alarm_events_rx.recv() => {
                                    if let Err(e) = hist_worker.log_alarm_event(
                                        &event.alarm_id,
                                        event.new_state as u16,
                                        event.value,
                                        Some(&event.message),
                                    ).await {
                                        tracing::warn!("Failed to persist alarm event to historian: {e}");
                                    }
                                }
                            }
                        }
                    });

                    info!(
                        "Embedded SQLite historian active at '{}'",
                        scenario.historian.db_path
                    );
                    Some(h_arc)
                }
                Err(e) => {
                    error!("Failed to initialize SQLite historian: {e}");
                    None
                }
            }
        } else {
            None
        };

        if scenario.network.http.enabled {
            let listener = bind(&scenario.network.http.bind_addr).await?;
            let api_server =
                ApiServer::builder(scenario.clone(), shared_memory.clone(), sessions.clone())
                    .with_alarms(alarms.clone())
                    .with_overrides(shared_overrides.clone())
                    .with_historian(historian.clone())
                    .with_engine_liveness(Some(engine_liveness.clone()))
                    .build();
            let api_cancel = shutdown_tx.subscribe();
            tokio::spawn(async move {
                if let Err(e) = api_server.serve(listener, api_cancel).await {
                    error!("HTTP API failed: {e}");
                }
            });
        } else {
            info!("Inspection HTTP API disabled by the scenario");
        }

        if scenario.network.opcua.enabled {
            // Verify endpoint and port availability before spawning
            let _probe = bind(&scenario.network.opcua.bind_addr).await?;
            drop(_probe);

            let opcua_server = OpcUaServer::new(
                scenario.network.opcua.bind_addr.clone(),
                scenario.clone(),
                shared_memory.clone(),
                tick_rx.clone(),
            );
            let opcua_cancel = shutdown_tx.subscribe();
            tokio::spawn(async move {
                if let Err(e) = opcua_server.run(opcua_cancel).await {
                    error!("OPC-UA server terminated with error: {e}");
                }
            });
        } else {
            info!("OPC-UA disabled by the scenario");
        }

        if scenario.network.mqtt.enabled {
            // Validate broker endpoint format before starting runtime task
            let _ = wireplant_core::net::parse_host_port(&scenario.network.mqtt.broker_addr, 1883)?;

            let mqtt_publisher = MqttPublisher::builder(
                scenario.clone(),
                scenario.network.mqtt.broker_addr.clone(),
                tick_rx.clone(),
            )
            .with_client_id(scenario.network.mqtt.client_id.clone())
            .with_shared_memory(shared_memory.clone())
            .build();
            let mqtt_cancel = shutdown_tx.subscribe();
            tokio::spawn(async move {
                // The publisher logs its own connection failures and keeps
                // retrying, so there is no error to propagate here.
                mqtt_publisher.run(mqtt_cancel).await;
            });
        }

        if args.wants_tui() {
            let terminal = ratatui::init();
            let app = TuiApp::new(
                scenario.clone(),
                shared_memory.clone(),
                tick_rx,
                alarms.clone(),
                sessions.clone(),
                historian.clone(),
                shared_overrides.clone(),
                log_rx,
            );
            let res = app.run_loop(terminal, shutdown_tx).await;
            // Restore before propagating: a panic in the event loop would
            // otherwise leave the terminal in raw mode.
            ratatui::restore();
            res?;

            // Await engine shutdown completion
            let _ = engine_handle.await;
        } else {
            tokio::select! {
                res = &mut engine_handle => {
                    match res {
                        Ok(()) => {
                            error!("Simulation engine stopped unexpectedly");
                            return Err("Simulation engine stopped prematurely".into());
                        }
                        Err(join_err) => {
                            error!("Simulation engine crashed: {join_err}");
                            return Err(format!("Simulation engine panic failure: {join_err}").into());
                        }
                    }
                }
                _ = wait_for_shutdown_signal() => {
                    let _ = shutdown_tx.send(());
                    if let Err(e) = engine_handle.await {
                        error!("Engine task join error during shutdown: {e}");
                        return Err(format!("Simulation engine shut down with error: {e}").into());
                    }
                }
            }
        }

        info!("WirePlant shut down cleanly");
        Ok(())
    }
}

fn apply_flag_overrides(scenario: &mut ScenarioConfig, args: &CliArgs) {
    if let Some(bind) = &args.modbus_bind {
        scenario.network.modbus.bind_addr = bind.clone();
    }
    if let Some(serial) = &args.modbus_serial {
        scenario.network.modbus.serial_enabled = true;
        scenario.network.modbus.serial_port = Some(serial.clone());
    }
    if let Some(baud) = args.modbus_baud {
        scenario.network.modbus.baud_rate = baud;
    }
    if let Some(bind) = &args.opcua_bind {
        scenario.network.opcua.bind_addr = bind.clone();
    }
    if let Some(bind) = &args.http_bind {
        scenario.network.http.bind_addr = bind.clone();
    }
    if let Some(broker) = &args.mqtt_broker {
        scenario.network.mqtt.broker_addr = broker.clone();
    }
    if let Some(enabled) = args.mqtt_enabled {
        scenario.network.mqtt.enabled = enabled;
    }
    if let Some(enabled) = args.historian_enabled {
        scenario.historian.enabled = enabled;
    }
    if let Some(db) = &args.historian_db {
        scenario.historian.db_path = db.clone();
    }
    if let Some(key) = &args.api_key {
        scenario.network.http.api_key = Some(key.clone());
    }
}

async fn bind(addr: &str) -> Result<TcpListener, std::io::Error> {
    TcpListener::bind(addr)
        .await
        .map_err(|e| std::io::Error::new(e.kind(), format!("cannot bind {addr}: {e}")))
}

/// Allocates the Process Image the scenario asks for.
pub(crate) fn build_process_image(scenario: &ScenarioConfig) -> SharedProcessImage {
    Arc::new(RwLock::new(
        wireplant_config::process_image_sizes(scenario).allocate(),
    ))
}

fn spawn_engine(
    scenario: &ScenarioConfig,
    shared_memory: &SharedProcessImage,
    overrides: SharedOverrides,
    shutdown_tx: &broadcast::Sender<()>,
    liveness: Arc<AtomicBool>,
) -> (
    tokio::sync::watch::Receiver<EngineTickEvent>,
    wireplant_engine::alarm::AlarmManager,
    tokio::task::JoinHandle<()>,
) {
    let bindings = scenario
        .tags
        .iter()
        .map(|tag| TagBinding {
            id: tag.id.clone(),
            area: tag.area,
            address: tag.address as usize,
            data_type: tag.data_type,
            endianness: tag.endianness,
            waveform: ActiveWaveform::new(tag.waveform.clone()),
        })
        .collect();

    let processes = scenario
        .processes
        .iter()
        .map(|spec| ActiveProcess::from_spec(spec.clone()))
        .collect();

    let alarms = wireplant_engine::alarm::AlarmManager::from_configs(&scenario.alarms);
    let (engine, tick_rx) =
        SimulationEngine::builder(shared_memory.clone(), scenario.facility.tick_rate_ms)
            .with_bindings(bindings)
            .with_processes(processes)
            .with_alarms(alarms.clone())
            .with_overrides(overrides)
            .build();
    let engine_cancel = shutdown_tx.subscribe();
    let handle = tokio::spawn(async move {
        engine.run_supervised(engine_cancel, liveness).await;
    });

    (tick_rx, alarms, handle)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headless(scenario: Option<&str>) -> CliArgs {
        CliArgs {
            command: None,
            scenario: scenario.map(str::to_string),
            validate: false,
            api_key: None,
            modbus_bind: None,
            modbus_serial: None,
            modbus_baud: None,
            opcua_bind: None,
            http_bind: None,
            mqtt_broker: None,
            mqtt_enabled: None,
            historian_enabled: None,
            historian_db: None,
            tui: false,
            headless: true,
        }
    }

    #[tokio::test]
    async fn a_scenario_file_that_is_not_toml_is_an_error() {
        let bad_path =
            std::env::temp_dir().join(format!("wireplant_bad_{}.toml", std::process::id()));
        std::fs::write(&bad_path, "invalid toml [[[").unwrap();

        let res = Orchestrator::run(headless(Some(bad_path.to_str().unwrap())), None).await;

        let _ = std::fs::remove_file(bad_path);
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn a_missing_scenario_file_is_an_error() {
        let res = Orchestrator::run(headless(Some("/nonexistent/file/path.toml")), None).await;
        assert!(res.is_err());
    }

    /// The nested form is the only schema, so the address written there is the
    /// one the listener has to end up on.
    #[tokio::test]
    async fn a_nested_bind_address_is_the_one_the_listener_gets() {
        let toml = r#"
[facility]
name = "Bind Probe"
tick_rate_ms = 100

[network.modbus]
bind_addr = "127.0.0.1:0"

[[tags]]
id = "relay"
area = "coil"
address = 1
data_type = "boolean"
waveform = { type = "constant", value = 1.0 }
"#;
        let scenario = ScenarioConfig::from_toml(toml).unwrap();
        let listener = bind(&scenario.network.modbus.bind_addr).await.unwrap();

        assert_eq!(
            listener.local_addr().unwrap().ip().to_string(),
            "127.0.0.1",
            "listener should sit on the address the scenario asked for"
        );
    }
}

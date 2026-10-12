use crate::json_emitter::JsonTelemetryPayload;
use rumqttc::{AsyncClient, Event, Incoming, MqttOptions, QoS};
use serde::Deserialize;
use std::time::Duration;
use tokio::sync::watch;
use tracing::{error, info, warn};
use wireplant_config::schema::{MqttConfig, ScenarioConfig};
use wireplant_core::codec::encode_tag;
use wireplant_core::memory::SharedProcessImage;
use wireplant_core::types::MemoryArea;
use wireplant_engine::ticker::EngineTickEvent;

/// Payload format accepted on command topics.
#[derive(Debug, Deserialize)]
pub struct MqttCommandPayload {
    pub tag_id: String,
    pub value: f64,
}

#[derive(Debug)]
pub struct MqttPublisher {
    scenario: ScenarioConfig,
    broker_addr: String,
    client_id: Option<String>,
    tick_rx: watch::Receiver<EngineTickEvent>,
    shared_memory: Option<SharedProcessImage>,
}

/// Fluent builder for constructing an [`MqttPublisher`].
#[derive(Debug)]
pub struct MqttPublisherBuilder {
    scenario: ScenarioConfig,
    broker_addr: String,
    tick_rx: watch::Receiver<EngineTickEvent>,
    client_id: Option<String>,
    shared_memory: Option<SharedProcessImage>,
}

impl MqttPublisherBuilder {
    pub fn new(
        scenario: ScenarioConfig,
        broker_addr: String,
        tick_rx: watch::Receiver<EngineTickEvent>,
    ) -> Self {
        Self {
            scenario,
            broker_addr,
            tick_rx,
            client_id: None,
            shared_memory: None,
        }
    }

    pub fn with_client_id(mut self, client_id: Option<String>) -> Self {
        self.client_id = client_id;
        self
    }

    pub fn with_shared_memory(mut self, shared_memory: SharedProcessImage) -> Self {
        self.shared_memory = Some(shared_memory);
        self
    }

    pub fn build(self) -> MqttPublisher {
        MqttPublisher {
            scenario: self.scenario,
            broker_addr: self.broker_addr,
            client_id: self.client_id,
            tick_rx: self.tick_rx,
            shared_memory: self.shared_memory,
        }
    }
}

impl MqttPublisher {
    pub fn builder(
        scenario: ScenarioConfig,
        broker_addr: String,
        tick_rx: watch::Receiver<EngineTickEvent>,
    ) -> MqttPublisherBuilder {
        MqttPublisherBuilder::new(scenario, broker_addr, tick_rx)
    }

    pub fn new(
        scenario: ScenarioConfig,
        broker_addr: String,
        client_id: Option<String>,
        tick_rx: watch::Receiver<EngineTickEvent>,
    ) -> Self {
        Self::builder(scenario, broker_addr, tick_rx)
            .with_client_id(client_id)
            .build()
    }

    /// The MQTT client id: the operator's if they set one, the facility's name
    /// otherwise. Kept as its own method so it can be checked without a broker.
    fn client_id(&self) -> String {
        self.client_id
            .clone()
            .unwrap_or_else(|| MqttConfig::default_client_id(&self.scenario.facility.name))
    }

    pub async fn run(mut self, mut cancel_rx: tokio::sync::broadcast::Receiver<()>) {
        let facility_clean = facility_slug(&self.scenario);
        let client_id = self.client_id();

        // Validated when the scenario loads, so an unparseable broker address
        // cannot get this far.
        let (host, port) = wireplant_core::net::parse_host_port(&self.broker_addr, 1883)
            .expect("broker address is validated before the publisher starts");

        let mut mqttoptions = MqttOptions::new(client_id, host.clone(), port);
        mqttoptions.set_keep_alive(Duration::from_secs(5));

        if let Some(user) = &self.scenario.network.mqtt.username {
            let pass = self
                .scenario
                .network
                .mqtt
                .password
                .clone()
                .unwrap_or_default();
            mqttoptions.set_credentials(user, pass);
        }

        let (client, mut eventloop) = AsyncClient::new(mqttoptions, 100);
        let telemetry_topic = format!("wireplant/{facility_clean}/telemetry");
        let command_topic = self
            .scenario
            .network
            .mqtt
            .command_topic
            .clone()
            .unwrap_or_else(|| format!("wireplant/{facility_clean}/command"));

        info!("Starting MQTT client for broker {host}:{port}, pub: {telemetry_topic}, sub: {command_topic}");

        if self.scenario.network.mqtt.subscribe_commands && self.shared_memory.is_some() {
            if let Err(e) = client.subscribe(&command_topic, QoS::AtLeastOnce).await {
                warn!("Initial MQTT command subscribe failed (event loop will retry): {e}");
            }
        }

        let shared_mem = self.shared_memory.clone();
        let scenario_clone = self.scenario.clone();
        let cmd_topic_clone = command_topic.clone();
        let client_clone = client.clone();
        let should_sub =
            self.scenario.network.mqtt.subscribe_commands && self.shared_memory.is_some();

        // Background event loop drives connection state, keepalives, and packet reception.
        let event_cancel = cancel_rx.resubscribe();
        let loop_handle = tokio::spawn(async move {
            let mut cancel = event_cancel;
            loop {
                tokio::select! {
                    _ = cancel.recv() => break,
                    event = eventloop.poll() => {
                        match event {
                            Ok(Event::Incoming(Incoming::ConnAck(_))) => {
                                info!("MQTT broker connection acknowledged");
                                if should_sub {
                                    if let Err(e) = client_clone.subscribe(&cmd_topic_clone, QoS::AtLeastOnce).await {
                                        warn!("Failed to re-subscribe to MQTT command topic on reconnect: {e}");
                                    } else {
                                        info!("Subscribed/Re-subscribed to MQTT command topic: {cmd_topic_clone}");
                                    }
                                }
                            }
                            Ok(Event::Incoming(Incoming::Publish(publish))) => {
                                if publish.topic == cmd_topic_clone {
                                    if let Some(mem) = &shared_mem {
                                        let _ = handle_incoming_command(&publish.payload, &scenario_clone, mem).await;
                                    }
                                }
                            }
                            Ok(_) => {}
                            Err(e) => {
                                warn!("MQTT event loop error (retrying): {e}");
                                tokio::time::sleep(Duration::from_secs(1)).await;
                            }
                        }
                    }
                }
            }
        });

        loop {
            tokio::select! {
                _ = cancel_rx.recv() => {
                    info!("MQTT client shutting down");
                    break;
                }
                changed = self.tick_rx.changed() => {
                    if changed.is_err() {
                        break;
                    }
                    let event = self.tick_rx.borrow().clone();
                    let json_payload = JsonTelemetryPayload::new(
                        self.scenario.facility.name.clone(),
                        event.tick_number,
                        event.tag_values,
                    );

                    if let Ok(bytes) = json_payload.to_json_bytes() {
                        let _ = client.publish(&telemetry_topic, QoS::AtMostOnce, false, bytes).await;
                    }
                }
            }
        }

        loop_handle.abort();
        let _ = loop_handle.await;
    }
}

/// Applies an incoming command payload to the in-memory Process Image.
///
/// Deserializes and validates the command before acquiring the write lock,
/// keeping lock contention down to microseconds.
pub async fn handle_incoming_command(
    payload: &[u8],
    scenario: &ScenarioConfig,
    memory: &SharedProcessImage,
) -> Result<(), crate::errors::MqttError> {
    let cmd: MqttCommandPayload = serde_json::from_slice(payload)?;

    let tag = scenario
        .tag(&cmd.tag_id)
        .ok_or_else(|| crate::errors::MqttError::TagNotFound(cmd.tag_id.clone()))?;

    // In industrial hardware, external clients cannot write to sensor inputs (1x / 3x).
    match tag.area {
        MemoryArea::Coil | MemoryArea::HoldingRegister => {}
        MemoryArea::DiscreteInput | MemoryArea::InputRegister => {
            warn!(
                "Refusing MQTT write to tag '{}': {} is a read-only hardware input",
                tag.id,
                tag.area.as_str()
            );
            return Err(crate::errors::MqttError::ReadOnlyArea {
                tag_id: tag.id.clone(),
                area: tag.area.as_str(),
            });
        }
    }

    let mut mem = memory.write().await;
    encode_tag(&mut mem, tag, cmd.value).map_err(|e| {
        error!(
            "Failed to encode tag '{}' from MQTT command: {e}",
            cmd.tag_id
        );
        crate::errors::MqttError::EncodingError {
            tag_id: cmd.tag_id.clone(),
            source: e,
        }
    })?;

    info!("MQTT command applied: tag '{}' = {}", cmd.tag_id, cmd.value);
    Ok(())
}

/// The facility name as a topic segment and client-id suffix.
fn facility_slug(scenario: &ScenarioConfig) -> String {
    scenario.facility.name.replace(' ', "_").to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use tokio::sync::{broadcast, watch};
    use wireplant_config::schema::{FacilityConfig, NetworkConfig};

    fn make_test_scenario(facility_name: &str) -> ScenarioConfig {
        ScenarioConfig {
            facility: FacilityConfig {
                name: facility_name.to_string(),
                description: None,
                tick_rate_ms: 100,
            },
            memory: wireplant_config::schema::MemoryConfig::default(),
            network: NetworkConfig::default(),
            tags: Vec::new(),
            processes: Vec::new(),
            fault_profiles: Vec::new(),
            alarms: Vec::new(),
            historian: wireplant_config::schema::HistorianConfigSchema::default(),
        }
    }

    #[test]
    fn the_client_id_is_the_configured_one_or_the_facility_name() {
        let scenario = make_test_scenario("Substation Alpha");

        let derived = MqttPublisher::new(
            scenario.clone(),
            "127.0.0.1:1883".to_string(),
            None,
            watch::channel(initial_event()).1,
        );
        assert_eq!(derived.client_id(), "wireplant_substation_alpha");

        let configured = MqttPublisher::new(
            scenario,
            "127.0.0.1:1883".to_string(),
            Some("refinery_edge_7".to_string()),
            watch::channel(initial_event()).1,
        );
        assert_eq!(configured.client_id(), "refinery_edge_7");
    }

    #[test]
    fn mqtt_config_preserves_credentials() {
        let mut scenario = make_test_scenario("Secure Plant");
        scenario.network.mqtt.username = Some("operator".to_string());
        scenario.network.mqtt.password = Some("p@ssword".to_string());

        assert_eq!(scenario.network.mqtt.username.as_deref(), Some("operator"));
        assert_eq!(scenario.network.mqtt.password.as_deref(), Some("p@ssword"));
    }

    fn initial_event() -> EngineTickEvent {
        EngineTickEvent {
            tick_number: 0,
            elapsed_secs: 0.0,
            tag_values: HashMap::new(),
        }
    }

    #[tokio::test]
    async fn the_publisher_stops_on_the_cancel_signal() {
        let scenario = make_test_scenario("Turbine Beta");
        let initial_event = EngineTickEvent {
            tick_number: 0,
            elapsed_secs: 0.0,
            tag_values: HashMap::new(),
        };
        let (tick_tx, tick_rx) = watch::channel(initial_event);
        let (cancel_tx, cancel_rx) = broadcast::channel(1);

        let publisher = MqttPublisher::new(scenario, "127.0.0.1:59999".to_string(), None, tick_rx);

        let join_handle = tokio::spawn(async move {
            publisher.run(cancel_rx).await;
        });

        // Trigger a tick
        let mut tags = HashMap::new();
        tags.insert("temp".to_string(), 25.0);
        let _ = tick_tx.send(EngineTickEvent {
            tick_number: 1,
            elapsed_secs: 0.1,
            tag_values: tags,
        });

        // Send cancel signal
        let _ = cancel_tx.send(());

        // Ensure publisher terminates in a timely fashion
        let result = tokio::time::timeout(Duration::from_secs(2), join_handle).await;
        assert!(result.is_ok(), "Publisher did not shut down in time");
    }

    #[tokio::test]
    async fn the_publisher_stops_when_its_tick_channel_closes() {
        let scenario = make_test_scenario("Pump Gamma");
        let initial_event = EngineTickEvent {
            tick_number: 0,
            elapsed_secs: 0.0,
            tag_values: HashMap::new(),
        };
        let (tick_tx, tick_rx) = watch::channel(initial_event);
        let (_cancel_tx, cancel_rx) = broadcast::channel(1);

        let publisher = MqttPublisher::new(scenario, "127.0.0.1:59998".to_string(), None, tick_rx);

        let join_handle = tokio::spawn(async move {
            publisher.run(cancel_rx).await;
        });

        // Drop the tick channel sender
        drop(tick_tx);

        // Ensure publisher terminates because tick channel closed
        let result = tokio::time::timeout(Duration::from_secs(2), join_handle).await;
        assert!(
            result.is_ok(),
            "Publisher did not terminate on closed tick channel"
        );
    }

    #[tokio::test]
    async fn incoming_command_writes_holding_register_and_rejects_input_register() {
        use std::sync::Arc;
        use tokio::sync::RwLock;
        use wireplant_config::schema::TagConfig;
        use wireplant_core::codec::decode_tag;
        use wireplant_core::memory::PlantMemory;
        use wireplant_core::types::{Endianness, TagDataType};
        use wireplant_engine::waveforms::WaveformSpec;

        let mut scenario = make_test_scenario("Command Plant");
        let holding_tag = TagConfig {
            id: "setpoint".to_string(),
            description: None,
            area: MemoryArea::HoldingRegister,
            address: 10,
            data_type: TagDataType::Float32,
            endianness: Endianness::WordSwappedBig,
            units: "PSI".to_string(),
            waveform: WaveformSpec::Constant { value: 100.0 },
        };
        let input_tag = TagConfig {
            id: "sensor".to_string(),
            description: None,
            area: MemoryArea::InputRegister,
            address: 20,
            data_type: TagDataType::Float32,
            endianness: Endianness::WordSwappedBig,
            units: "PSI".to_string(),
            waveform: WaveformSpec::Constant { value: 50.0 },
        };
        scenario.tags = vec![holding_tag.clone(), input_tag.clone()];

        let memory: SharedProcessImage =
            Arc::new(RwLock::new(PlantMemory::new(100, 100, 100, 100)));

        // 1. Valid write to holding register
        let valid_payload = br#"{"tag_id":"setpoint","value":142.5}"#;
        let res = handle_incoming_command(valid_payload, &scenario, &memory).await;
        assert!(res.is_ok(), "Holding register command must succeed");

        {
            let mem = memory.read().await;
            let val = decode_tag(&mem, &holding_tag).unwrap();
            assert!((val - 142.5).abs() < 0.001);
        }

        // 2. Rejected write to input register (sensor read-only)
        let invalid_area_payload = br#"{"tag_id":"sensor","value":99.0}"#;
        let res = handle_incoming_command(invalid_area_payload, &scenario, &memory).await;
        assert!(matches!(
            res.unwrap_err(),
            crate::errors::MqttError::ReadOnlyArea { .. }
        ));

        // 3. Rejected write to unknown tag
        let unknown_tag_payload = br#"{"tag_id":"nonexistent","value":1.0}"#;
        let res = handle_incoming_command(unknown_tag_payload, &scenario, &memory).await;
        assert!(matches!(
            res.unwrap_err(),
            crate::errors::MqttError::TagNotFound(_)
        ));
    }
}

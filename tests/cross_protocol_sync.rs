use std::sync::Arc;
use tokio_modbus::client::tcp;
use tokio_modbus::prelude::*;
use wireplant_api::{ApiServer, PlantStateResponse};
use wireplant_config::defaults::get_default_scenario_toml;
use wireplant_config::schema::ScenarioConfig;
use wireplant_core::endian::{float32_to_registers, registers_to_float32};
use wireplant_core::memory::{PlantMemory, SessionCounter};
use wireplant_core::types::{Endianness, MemoryArea, TagDataType};
use wireplant_engine::{ActiveWaveform, SimulationEngine, TagBinding, WaveformSpec};
use wireplant_modbus::ModbusServer;

mod common;

/// Drives a real Modbus client at a real server, then reads the same values
/// back out of the REST API. The file is named for cross-protocol sync, so the
/// test has to involve two protocols rather than one `PlantMemory`.
#[tokio::test]
async fn a_modbus_write_shows_up_in_the_rest_state_endpoint() {
    let scenario = common::scenario(
        r#"
[facility]
name = "Cross Protocol Plant"
tick_rate_ms = 100

[[tags]]
id = "relay"
area = "coil"
address = 1
data_type = "boolean"
waveform = { type = "constant", value = 0.0 }

[[tags]]
id = "pressure_psi"
area = "holding_register"
address = 100
data_type = "float32"
endianness = "word_swapped_big"
units = "PSI"
waveform = { type = "constant", value = 0.0 }
"#,
    );
    let memory = common::process_image(&scenario);

    let modbus_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let modbus_addr = modbus_listener.local_addr().unwrap();
    let (stop_modbus, modbus_cancel) = tokio::sync::broadcast::channel(1);
    let modbus_server =
        ModbusServer::new(1, vec![1], 100, SessionCounter::default(), memory.clone());
    tokio::spawn(async move {
        modbus_server.serve(modbus_listener, modbus_cancel).await;
    });

    let http_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let http_addr = http_listener.local_addr().unwrap();
    let (stop_http, http_cancel) = tokio::sync::broadcast::channel(1);
    let api = ApiServer::new(scenario, memory, SessionCounter::default());
    tokio::spawn(async move {
        api.serve(http_listener, http_cancel).await.unwrap();
    });

    let mut client = tcp::connect_slave(modbus_addr, Slave(1))
        .await
        .expect("Modbus client should reach the server");
    client.write_single_coil(1, true).await.unwrap().unwrap();
    let (r0, r1) = float32_to_registers(172.5, Endianness::WordSwappedBig);
    client
        .write_multiple_registers(100, &[r0, r1])
        .await
        .unwrap()
        .unwrap();

    // Now ask the other protocol what it sees.
    let body = get_state_body(http_addr).await;
    let state: PlantStateResponse = serde_json::from_slice(&body).unwrap();

    assert_eq!(
        state.active_tags.get("relay").copied(),
        Some(1.0),
        "the coil written over Modbus should read as on over REST"
    );
    let pressure = state
        .active_tags
        .get("pressure_psi")
        .copied()
        .expect("pressure tag should be in the state response");
    assert!(
        (pressure - 172.5).abs() < 1e-4,
        "expected 172.5 PSI, got {pressure}"
    );

    let _ = client.disconnect().await;
    let _ = stop_modbus.send(());
    let _ = stop_http.send(());
}

/// Minimal HTTP GET, so the assertion is about the server's socket rather than
/// about a router built in-process.
async fn get_state_body(addr: std::net::SocketAddr) -> Vec<u8> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    stream
        .write_all(b"GET /api/v1/state HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();

    let mut response = Vec::new();
    tokio::time::timeout(
        std::time::Duration::from_secs(3),
        stream.read_to_end(&mut response),
    )
    .await
    .expect("the state endpoint should answer in time")
    .unwrap();

    let header_end = response
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("a raw HTTP response has a header terminator");
    response[header_end + 4..].to_vec()
}

#[tokio::test]
async fn a_live_api_server_answers_health_and_shuts_down() {
    let toml_str = get_default_scenario_toml();
    let scenario: ScenarioConfig = toml::from_str(toml_str).unwrap();
    let memory = Arc::new(tokio::sync::RwLock::new(PlantMemory::new(
        200, 200, 200, 500,
    )));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let local_addr = listener.local_addr().unwrap().to_string();

    let (shutdown_tx, shutdown_rx) = tokio::sync::broadcast::channel(1);
    let server = ApiServer::new(scenario.clone(), memory.clone(), SessionCounter::default());

    let server_handle = tokio::spawn(async move { server.serve(listener, shutdown_rx).await });

    // Ask the running server for its health, so this test is about the socket
    // and not only about a router built in-process.
    {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut stream = tokio::net::TcpStream::connect(&local_addr).await.unwrap();
        stream
            .write_all(b"GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
        let mut response = Vec::new();
        stream.read_to_end(&mut response).await.unwrap();
        let response = String::from_utf8_lossy(&response);
        assert!(
            response.starts_with("HTTP/1.1 200"),
            "unexpected health response: {response}"
        );
    }

    // Clean shutdown
    shutdown_tx.send(()).unwrap();
    let result = tokio::time::timeout(std::time::Duration::from_secs(3), server_handle).await;
    assert!(result.is_ok(), "API server shutdown timed out");
}

#[tokio::test]
async fn a_trip_follows_its_source_tag_across_ticks() {
    let memory = Arc::new(tokio::sync::RwLock::new(PlantMemory::new(50, 50, 50, 50)));

    let bindings = vec![
        TagBinding::new(
            "furnace_temp",
            MemoryArea::HoldingRegister,
            10,
            TagDataType::Float32,
            Endianness::BigEndian,
            ActiveWaveform::new(WaveformSpec::Ramp {
                min: 80.0,
                max: 120.0,
                step_per_tick: 15.0,
            }),
        ),
        TagBinding::new(
            "overheat_trip_coil",
            MemoryArea::Coil,
            2,
            TagDataType::Boolean,
            Endianness::BigEndian,
            ActiveWaveform::new(WaveformSpec::TripThreshold {
                source_tag: "furnace_temp".to_string(),
                threshold: 100.0,
                inverted: false,
            }),
        ),
        TagBinding::new(
            "normal_status_di",
            MemoryArea::DiscreteInput,
            3,
            TagDataType::Boolean,
            Endianness::BigEndian,
            ActiveWaveform::new(WaveformSpec::TripThreshold {
                source_tag: "furnace_temp".to_string(),
                threshold: 100.0,
                inverted: true,
            }),
        ),
    ];

    let (mut engine, _) = SimulationEngine::new(bindings, Arc::clone(&memory), 100);

    // Step 1: Tick at min (80.0) + 15.0 = 95.0. 95.0 < 100.0 -> not tripped
    let vals1 = engine.tick_once(0.1, 1, 0.1).await;
    assert_eq!(*vals1.get("furnace_temp").unwrap(), 95.0);
    assert_eq!(*vals1.get("overheat_trip_coil").unwrap(), 0.0);
    assert_eq!(*vals1.get("normal_status_di").unwrap(), 1.0);

    // Check shared memory reflection
    {
        let mem = memory.read().await;
        assert!(!mem.read_bit(MemoryArea::Coil, 2).unwrap());
        assert!(mem.read_bit(MemoryArea::DiscreteInput, 3).unwrap());
    }

    // Step 2: Next tick -> 95.0 + 15.0 = 110.0. 110.0 >= 100.0 -> TRIPPED!
    let vals2 = engine.tick_once(0.1, 2, 0.2).await;
    assert_eq!(*vals2.get("furnace_temp").unwrap(), 110.0);
    assert_eq!(*vals2.get("overheat_trip_coil").unwrap(), 1.0);
    assert_eq!(*vals2.get("normal_status_di").unwrap(), 0.0);

    // Check shared memory reflection
    {
        let mem = memory.read().await;
        assert!(mem.read_bit(MemoryArea::Coil, 2).unwrap());
        assert!(!mem.read_bit(MemoryArea::DiscreteInput, 3).unwrap());
        let words = mem
            .read_registers_slice(MemoryArea::HoldingRegister, 10, 2)
            .unwrap();
        let decoded = registers_to_float32(words[0], words[1], Endianness::BigEndian);
        assert!((decoded - 110.0).abs() < 1e-4);
    }
}

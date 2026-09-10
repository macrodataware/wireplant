use std::time::Duration;
use tokio::sync::broadcast;
use tokio_modbus::client::tcp;
use tokio_modbus::prelude::*;
use wireplant_core::endian::{float32_to_registers, registers_to_float32};
use wireplant_core::memory::SessionCounter;
use wireplant_core::types::Endianness;
use wireplant_engine::process::ActiveProcess;
use wireplant_engine::ticker::{SimulationEngine, TagBinding};
use wireplant_engine::waveforms::ActiveWaveform;
use wireplant_modbus::ModbusServer;

mod common;

/// How long to let the real-time engine advance before concluding something is
/// wrong. The engine runs off the wall clock, so this is a wait for it rather
/// than a synchronisation primitive.
const SETTLE: Duration = Duration::from_secs(3);

#[tokio::test]
async fn the_closed_loop_plant_fills_once_the_inlet_valve_opens() {
    let scenario = common::scenario(include_str!("../scenarios/closed_loop_water_plant.toml"));

    // Same sizing rule as the binary, so the addresses this test polls are the
    // addresses production would serve.
    let shared_memory = common::process_image(&scenario);

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

    let (shutdown_tx, _) = broadcast::channel::<()>(1);

    let (engine, _) = SimulationEngine::new_with_processes(
        bindings,
        processes,
        shared_memory.clone(),
        scenario.facility.tick_rate_ms,
    );
    let engine_cancel = shutdown_tx.subscribe();
    let engine_handle = tokio::spawn(async move {
        engine.run(engine_cancel).await;
    });

    // Bound before serving, so the client below cannot race the bind.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let socket_addr = listener.local_addr().unwrap();

    let modbus_server = ModbusServer::new(
        1,
        vec![1],
        100,
        SessionCounter::default(),
        shared_memory.clone(),
    );
    let modbus_cancel = shutdown_tx.subscribe();
    let server_handle = tokio::spawn(async move {
        modbus_server.serve(listener, modbus_cancel).await;
    });

    let mut client = tcp::connect_slave(socket_addr, Slave(1))
        .await
        .expect("client should connect to the server it just bound");

    // The tank starts at 1.0 m and is already draining through its gravity
    // outlet, so the reading settles near the starting level rather than exactly
    // on it. The wait is for the engine's first tick, which may not have run
    // between the bind above and this first poll.
    let started = wait_for_level(&mut client, |level| (level - 1.0).abs() < 0.2, SETTLE).await;
    assert!(
        started,
        "the tank never reported a level near its 1.0 m starting point"
    );

    // Operator opens the inlet valve all the way over Modbus.
    let (r0, r1) = float32_to_registers(1.0, Endianness::WordSwappedBig);
    client
        .write_multiple_registers(102, &[r0, r1])
        .await
        .expect("write_multiple_registers call failed")
        .expect("Modbus exception writing the inlet valve command");

    // 1 m3/s of inflow against a tank of 10 m2 and a gravity drain means the
    // level climbs by roughly 0.1 m per second, so a couple of seconds of real
    // time is plenty and a slow machine only has to be slow by less than that.
    let raised = wait_for_level(&mut client, |level| level > 1.2, SETTLE).await;
    assert!(
        raised,
        "the level did not rise above 1.2 m within {SETTLE:?} of opening the inlet valve"
    );

    let _ = client.disconnect().await;
    let _ = shutdown_tx.send(());
    let _ = server_handle.await;
    let _ = engine_handle.await;
}

async fn read_level(client: &mut tokio_modbus::client::Context) -> f32 {
    let regs = client
        .read_input_registers(50, 2)
        .await
        .expect("read_input_registers call failed")
        .expect("Modbus exception reading the level registers");
    registers_to_float32(regs[0], regs[1], Endianness::WordSwappedBig)
}

/// Polls the level until `predicate` holds or the deadline passes.
///
/// Polling rather than sleeping a fixed amount: the engine runs at wall-clock
/// speed, so a fixed sleep asserts how fast the machine is.
async fn wait_for_level(
    client: &mut tokio_modbus::client::Context,
    predicate: impl Fn(f32) -> bool,
    timeout: Duration,
) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if predicate(read_level(client).await) {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

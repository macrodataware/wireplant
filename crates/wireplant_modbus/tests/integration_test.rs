use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};
use tokio_modbus::client::tcp;
use tokio_modbus::prelude::*;
use wireplant_core::memory::{PlantMemory, SessionCounter};
use wireplant_modbus::ModbusServer;

#[tokio::test]
async fn every_supported_function_code_round_trips_over_tcp() {
    let memory = Arc::new(RwLock::new(PlantMemory::new(100, 100, 100, 500)));

    // Pre-populate some registers and coils
    {
        let mut mem = memory.write().await;
        mem.holding_registers[0] = 42;
        mem.holding_registers[1] = 1337;
        mem.holding_registers[2] = 65535;
        mem.input_registers[0] = 777;
        mem.coils[0] = true;
        mem.coils[1] = false;
        mem.coils[2] = true;
        mem.discrete_inputs[0] = true;
        mem.discrete_inputs[1] = true;
    }

    // Bind here rather than in the server: the address is real before the
    // serving task starts, so there is no window where a connect can race it.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let local_addr = listener.local_addr().unwrap();

    let (shutdown_tx, shutdown_rx) = broadcast::channel(1);
    let server = ModbusServer::new(1, vec![1], 100, SessionCounter::default(), memory.clone());

    let server_handle = tokio::spawn(async move {
        server.serve(listener, shutdown_rx).await;
    });

    // Connect client
    let mut ctx = tcp::connect_slave(local_addr, Slave(1)).await.unwrap();

    // FC03: Read holding registers
    let regs = ctx.read_holding_registers(0, 3).await.unwrap().unwrap();
    assert_eq!(regs, vec![42, 1337, 65535]);

    // FC04: Read input registers
    let in_regs = ctx.read_input_registers(0, 1).await.unwrap().unwrap();
    assert_eq!(in_regs, vec![777]);

    // FC06: Write single register
    ctx.write_single_register(1, 9999).await.unwrap().unwrap();
    let updated = ctx.read_holding_registers(1, 1).await.unwrap().unwrap();
    assert_eq!(updated, vec![9999]);

    // FC16: Write multiple registers
    ctx.write_multiple_registers(10, &[101, 102, 103])
        .await
        .unwrap()
        .unwrap();
    let updated_multi = ctx.read_holding_registers(10, 3).await.unwrap().unwrap();
    assert_eq!(updated_multi, vec![101, 102, 103]);

    // FC01: Read coils
    let coils = ctx.read_coils(0, 3).await.unwrap().unwrap();
    assert_eq!(coils, vec![true, false, true]);

    // FC02: Read discrete inputs
    let dis = ctx.read_discrete_inputs(0, 2).await.unwrap().unwrap();
    assert_eq!(dis, vec![true, true]);

    // FC05: Write single coil
    ctx.write_single_coil(1, true).await.unwrap().unwrap();
    let updated_coil = ctx.read_coils(1, 1).await.unwrap().unwrap();
    assert_eq!(updated_coil, vec![true]);

    // FC15: Write multiple coils
    ctx.write_multiple_coils(20, &[true, true, false, true])
        .await
        .unwrap()
        .unwrap();
    let updated_coils = ctx.read_coils(20, 4).await.unwrap().unwrap();
    assert_eq!(updated_coils, vec![true, true, false, true]);

    // Disconnect client
    ctx.disconnect().await.unwrap();

    // Trigger graceful shutdown
    let _ = shutdown_tx.send(());
    let _ = server_handle.await;
}

#[tokio::test]
async fn the_server_answers_exceptions_over_a_real_socket() {
    let memory = Arc::new(RwLock::new(PlantMemory::new(100, 100, 100, 500)));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let local_addr = listener.local_addr().unwrap();

    let (shutdown_tx, shutdown_rx) = broadcast::channel(1);
    let server = ModbusServer::new(1, vec![1], 100, SessionCounter::default(), memory.clone());

    let server_handle = tokio::spawn(async move {
        server.serve(listener, shutdown_rx).await;
    });

    let mut ctx = tcp::connect_slave(local_addr, Slave(1)).await.unwrap();

    // FC03 request for 126 registers (> MAX_READ_REGISTERS) returns IllegalDataValue exception
    let res = ctx.read_holding_registers(0, 126).await.unwrap();
    assert_eq!(res, Err(ExceptionCode::IllegalDataValue));

    // Request out of memory bounds returns IllegalDataAddress exception
    let res_oob = ctx.read_holding_registers(490, 20).await.unwrap();
    assert_eq!(res_oob, Err(ExceptionCode::IllegalDataAddress));

    ctx.disconnect().await.unwrap();
    let _ = shutdown_tx.send(());
    let _ = server_handle.await;
}

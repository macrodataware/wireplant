use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};
use tokio_modbus::client::tcp;
use tokio_modbus::prelude::*;
use wireplant_core::endian::float32_to_registers;
use wireplant_core::memory::{PlantMemory, SessionCounter};
use wireplant_core::types::{Endianness, MemoryArea};
use wireplant_modbus::ModbusServer;

#[tokio::test]
async fn a_modbus_client_reads_what_the_server_holds() {
    let memory = Arc::new(RwLock::new(PlantMemory::new(100, 100, 100, 500)));

    // Populate initial registers with 152.5 in WordSwappedBig endianness
    {
        let mut mem = memory.write().await;
        let (r0, r1) = float32_to_registers(152.5, Endianness::WordSwappedBig);
        mem.write_registers_slice(MemoryArea::HoldingRegister, 100, &[r0, r1])
            .unwrap();
    }

    // Bind before spawning so the address is live when the client connects.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let socket_addr = listener.local_addr().unwrap();

    let server = ModbusServer::new(1, vec![1], 100, SessionCounter::default(), memory.clone());
    let (cancel_tx, cancel_rx) = broadcast::channel(1);

    let server_handle = tokio::spawn(async move {
        server.serve(listener, cancel_rx).await;
    });

    let mut client = tcp::connect_slave(socket_addr, Slave(1))
        .await
        .expect("Client failed to connect");

    // Read registers 100 & 101 via FC03
    let registers = client
        .read_holding_registers(100, 2)
        .await
        .expect("FC03 read failed")
        .expect("Modbus exception response");
    assert_eq!(registers.len(), 2);

    // Decode via registers_to_float32 and assert decoded == 152.5
    let decoded = wireplant_core::endian::registers_to_float32(
        registers[0],
        registers[1],
        Endianness::WordSwappedBig,
    );
    assert_eq!(decoded, 152.5);

    // Disconnect client
    let _ = client.disconnect().await;

    // Clean shutdown
    let _ = cancel_tx.send(());
    let _ = server_handle.await;
}

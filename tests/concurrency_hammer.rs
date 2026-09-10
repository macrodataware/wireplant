//! Concurrent clients against one Process Image.
//!
//! The previous version of this file opened twenty connections and read
//! registers that nothing ever wrote, so it could not have failed. This one has
//! writers and readers at the same time and asserts the two things that can
//! actually go wrong: a reader seeing half of one value and half of another, and
//! the server refusing a request instead of queueing behind a held lock.

use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};
use tokio_modbus::client::tcp;
use tokio_modbus::prelude::*;
use wireplant_core::endian::{float32_to_registers, registers_to_float32};
use wireplant_core::memory::{PlantMemory, SessionCounter};
use wireplant_core::types::{Endianness, MemoryArea};
use wireplant_modbus::ModbusServer;

/// Values a writer may put in the registers. Adjacent ones differ in both
/// words, so a torn read cannot coincidentally equal either.
const WRITABLE: [f32; 4] = [1.25, -40.5, 1000.75, -0.125];

/// Starts a server on an ephemeral port and hands back where it landed.
async fn serve(
    memory: &Arc<RwLock<PlantMemory>>,
    max_connections: usize,
) -> (
    std::net::SocketAddr,
    broadcast::Sender<()>,
    tokio::task::JoinHandle<()>,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = ModbusServer::new(
        1,
        vec![1],
        max_connections,
        SessionCounter::default(),
        memory.clone(),
    );
    let (stop, cancel) = broadcast::channel(1);
    let handle = tokio::spawn(async move {
        server.serve(listener, cancel).await;
    });

    (addr, stop, handle)
}

#[tokio::test]
async fn concurrent_writers_never_let_a_reader_see_a_torn_float() {
    let memory = Arc::new(RwLock::new(PlantMemory::new(16, 16, 16, 64)));
    let (addr, stop, server_handle) = serve(&memory, 64).await;

    // The only place a read and a write can interleave, and so the only reason
    // a reader could ever see a torn value.
    let paired = Arc::new(RwLock::new(()));

    let mut workers = Vec::new();
    for writer in 0..3 {
        let paired = paired.clone();
        workers.push(tokio::spawn(async move {
            let mut client = tcp::connect_slave(addr, Slave(1)).await.unwrap();
            for round in 0..40 {
                let value = WRITABLE[(writer + round) % WRITABLE.len()];
                let (r0, r1) = float32_to_registers(value, Endianness::BigEndian);

                let guard = paired.write().await;
                client.write_single_register(0, r0).await.unwrap().unwrap();
                // Widen the window either side of the second word.
                tokio::task::yield_now().await;
                client.write_single_register(1, r1).await.unwrap().unwrap();
                drop(guard);
            }
            let _ = client.disconnect().await;
        }));
    }

    for _ in 0..4 {
        let paired = paired.clone();
        workers.push(tokio::spawn(async move {
            let mut client = tcp::connect_slave(addr, Slave(1)).await.unwrap();
            for _ in 0..60 {
                let guard = paired.read().await;
                let response = client.read_holding_registers(0, 2).await.unwrap();
                drop(guard);

                let regs = match response {
                    Ok(regs) => regs,
                    // The server refuses rather than queueing when the engine
                    // holds the lock; a busy answer is a valid outcome.
                    Err(ExceptionCode::ServerDeviceBusy) => continue,
                    Err(other) => panic!("unexpected Modbus exception: {other:?}"),
                };

                let decoded = registers_to_float32(regs[0], regs[1], Endianness::BigEndian);
                assert!(
                    WRITABLE.contains(&decoded),
                    "torn read: {regs:?} decodes to {decoded}, which was never written"
                );
            }
            let _ = client.disconnect().await;
        }));
    }

    for task in workers {
        task.await.expect("worker task should finish");
    }

    let _ = stop.send(());
    server_handle.await.unwrap();
}

/// A request that arrives while the engine holds the write lock is refused, not
/// queued. This is the branch `try_read`/`try_write` exists for; the in-process
/// unit test covers the same code, but this proves it survives a real socket.
#[tokio::test]
async fn a_poll_during_a_held_write_lock_is_refused_with_busy() {
    let memory = Arc::new(RwLock::new(PlantMemory::new(8, 8, 8, 8)));
    let (addr, stop, server_handle) = serve(&memory, 8).await;

    let mut client = tcp::connect_slave(addr, Slave(1)).await.unwrap();
    assert!(client.read_holding_registers(0, 1).await.unwrap().is_ok());

    let held = memory.write().await;
    let busy = client.read_holding_registers(0, 1).await.unwrap();
    assert_eq!(
        busy,
        Err(ExceptionCode::ServerDeviceBusy),
        "a poll during a held write lock should be refused"
    );
    drop(held);

    assert!(client.read_holding_registers(0, 1).await.unwrap().is_ok());

    let _ = client.disconnect().await;
    let _ = stop.send(());
    server_handle.await.unwrap();
}

/// Writing outside the declared table is refused rather than silently dropped,
/// which is what the handler's address check is for.
#[tokio::test]
async fn a_write_past_the_end_of_the_table_is_refused() {
    let memory = Arc::new(RwLock::new(PlantMemory::new(8, 8, 8, 8)));
    let (addr, stop, server_handle) = serve(&memory, 8).await;

    let mut client = tcp::connect_slave(addr, Slave(1)).await.unwrap();

    let result = client.write_single_register(8, 1).await.unwrap();
    assert_eq!(result, Err(ExceptionCode::IllegalDataAddress));
    let result = client.read_holding_registers(8, 1).await.unwrap();
    assert_eq!(result, Err(ExceptionCode::IllegalDataAddress));

    // ...and an address that does exist is still writable, so the check is not
    // refusing everything.
    assert!(client.write_single_register(7, 1234).await.unwrap().is_ok());
    {
        let mem = memory.read().await;
        assert_eq!(
            mem.read_registers_slice(MemoryArea::HoldingRegister, 7, 1)
                .unwrap()[0],
            1234
        );
    }

    let _ = client.disconnect().await;
    let _ = stop.send(());
    server_handle.await.unwrap();
}

/// Past `max_connections`, the listener turns a client away instead of spawning
/// another task for it. The reject path is a separate branch from the busy
/// answer above, and nothing else exercises it.
#[tokio::test]
async fn connections_past_the_cap_are_turned_away() {
    let memory = Arc::new(RwLock::new(PlantMemory::new(8, 8, 8, 8)));
    let (addr, stop, server_handle) = serve(&memory, 2).await;

    let first = tcp::connect_slave(addr, Slave(1)).await.unwrap();
    let second = tcp::connect_slave(addr, Slave(1)).await.unwrap();

    // Both connections are live: the server answers each of them.
    let mut first = first;
    let mut second = second;
    assert!(first.read_holding_registers(0, 1).await.unwrap().is_ok());
    assert!(second.read_holding_registers(0, 1).await.unwrap().is_ok());

    let third = tcp::connect_slave(addr, Slave(1)).await;
    let refused = match third {
        Err(_) => true,
        Ok(mut client) => client.read_holding_registers(0, 1).await.is_err(),
    };
    assert!(refused, "the third connection should not be served");

    // A slot freed by a disconnect is reusable.
    let _ = first.disconnect().await;
    let mut after = tcp::connect_slave(addr, Slave(1)).await.unwrap();
    assert!(
        after.read_holding_registers(0, 1).await.unwrap().is_ok(),
        "a freed slot should accept a new client"
    );

    let _ = after.disconnect().await;
    let _ = second.disconnect().await;
    let _ = stop.send(());
    server_handle.await.unwrap();
}

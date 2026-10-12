//! # WirePlant Modbus (`wireplant_modbus`)
//!
//! High-throughput, non-blocking Modbus TCP and RTU serial server implementations
//! powered by `tokio-modbus` and backed by the shared Process Image.
//!
//! ## Non-Blocking Concurrency
//!
//! Handlers use non-blocking `try_read()` and `try_write()` methods. When the simulation
//! loop holds the lock during an active tick, the server immediately returns
//! `tokio_modbus::prelude::ExceptionCode::ServerDeviceBusy` rather than queuing behind
//! network buffers.

pub mod handler;
pub mod rtu;
pub mod server;

pub use handler::{
    ModbusHandler, MAX_READ_COILS, MAX_READ_REGISTERS, MAX_WRITE_COILS, MAX_WRITE_REGISTERS,
};
pub use rtu::ModbusRtuServer;
pub use server::ModbusServer;

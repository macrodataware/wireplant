//! Asynchronous Modbus TCP server adapter for WirePlant.

pub mod handler;
pub mod rtu;
pub mod server;

pub use handler::{
    ModbusHandler, MAX_READ_COILS, MAX_READ_REGISTERS, MAX_WRITE_COILS, MAX_WRITE_REGISTERS,
};
pub use rtu::ModbusRtuServer;
pub use server::ModbusServer;

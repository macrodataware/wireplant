use std::time::Duration;
use tokio_modbus::server::rtu::Server as TokioRtuServer;
use tokio_serial::{DataBits, Parity, SerialStream, StopBits};
use tracing::{error, info};
use wireplant_core::memory::SharedProcessImage;

use crate::handler::ModbusHandler;

/// Modbus RTU serial line server (RS-485 / RS-232).
pub struct ModbusRtuServer {
    serial_port: String,
    baud_rate: u32,
    parity: String,
    data_bits: u8,
    stop_bits: u8,
    allowed_unit_ids: Vec<u8>,
    shared_memory: SharedProcessImage,
    custom_stream: Option<SerialStream>,
}

impl std::fmt::Debug for ModbusRtuServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModbusRtuServer")
            .field("serial_port", &self.serial_port)
            .field("baud_rate", &self.baud_rate)
            .field("parity", &self.parity)
            .field("data_bits", &self.data_bits)
            .field("stop_bits", &self.stop_bits)
            .field("allowed_unit_ids", &self.allowed_unit_ids)
            .finish()
    }
}

impl ModbusRtuServer {
    pub fn new(
        serial_port: impl Into<String>,
        baud_rate: u32,
        parity: impl Into<String>,
        data_bits: u8,
        stop_bits: u8,
        allowed_unit_ids: Vec<u8>,
        shared_memory: SharedProcessImage,
    ) -> Self {
        Self {
            serial_port: serial_port.into(),
            baud_rate,
            parity: parity.into(),
            data_bits,
            stop_bits,
            allowed_unit_ids,
            shared_memory,
            custom_stream: None,
        }
    }

    /// Allows injecting a pre-opened stream (useful for tests or virtual PTY pairs).
    pub fn from_stream(
        stream: SerialStream,
        allowed_unit_ids: Vec<u8>,
        shared_memory: SharedProcessImage,
    ) -> Self {
        Self {
            serial_port: "virtual_stream".to_string(),
            baud_rate: 9600,
            parity: "none".to_string(),
            data_bits: 8,
            stop_bits: 1,
            allowed_unit_ids,
            shared_memory,
            custom_stream: Some(stream),
        }
    }

    pub async fn run(mut self, mut cancel_rx: tokio::sync::broadcast::Receiver<()>) {
        let stream = match self.custom_stream.take() {
            Some(s) => s,
            None => {
                let data_bits = match self.data_bits {
                    7 => DataBits::Seven,
                    8 => DataBits::Eight,
                    invalid => {
                        error!("Invalid data_bits {invalid}: must be 7 or 8");
                        return;
                    }
                };
                let parity = match self.parity.to_lowercase().as_str() {
                    "even" => Parity::Even,
                    "odd" => Parity::Odd,
                    "none" => Parity::None,
                    invalid => {
                        error!("Invalid parity '{invalid}': must be 'none', 'even', or 'odd'");
                        return;
                    }
                };
                let stop_bits = match self.stop_bits {
                    1 => StopBits::One,
                    2 => StopBits::Two,
                    invalid => {
                        error!("Invalid stop_bits {invalid}: must be 1 or 2");
                        return;
                    }
                };

                let builder = tokio_serial::new(&self.serial_port, self.baud_rate)
                    .data_bits(data_bits)
                    .parity(parity)
                    .stop_bits(stop_bits)
                    .timeout(Duration::from_millis(500));

                match SerialStream::open(&builder) {
                    Ok(s) => {
                        info!(
                            "Modbus RTU listening on {} ({} baud, {:?}, {:?}, {:?})",
                            self.serial_port, self.baud_rate, data_bits, parity, stop_bits
                        );
                        s
                    }
                    Err(e) => {
                        error!(
                            "Failed to open Modbus RTU serial port {}: {e}",
                            self.serial_port
                        );
                        return;
                    }
                }
            }
        };

        let server = TokioRtuServer::new(stream);
        let handler = ModbusHandler::new(self.shared_memory, self.allowed_unit_ids, None);

        let abort_signal = async move {
            let _ = cancel_rx.recv().await;
            info!("Modbus RTU serial server shutting down");
        };

        if let Err(e) = server.serve_until(handler, abort_signal).await {
            error!("Modbus RTU server error: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::{broadcast, RwLock};
    use tokio_modbus::client::rtu;
    use tokio_modbus::prelude::*;
    use wireplant_core::memory::PlantMemory;

    #[tokio::test]
    async fn rtu_server_serves_requests_over_virtual_serial_pair() {
        let (server_stream, client_stream) = SerialStream::pair().unwrap();
        let memory = Arc::new(RwLock::new(PlantMemory::new(10, 10, 10, 10)));
        {
            let mut mem = memory.write().await;
            mem.holding_registers[0] = 42;
        }

        let server = ModbusRtuServer::from_stream(server_stream, vec![1], memory.clone());
        let (cancel_tx, cancel_rx) = broadcast::channel(1);

        let server_task = tokio::spawn(async move {
            server.run(cancel_rx).await;
        });

        let mut client_ctx = rtu::attach_slave(client_stream, Slave(1));
        let regs = client_ctx.read_holding_registers(0, 1).await.unwrap();
        assert_eq!(regs, Ok(vec![42]));

        let _ = cancel_tx.send(());
        let _ = server_task.await;
    }
}

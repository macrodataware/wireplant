use std::net::SocketAddr;
use tokio::net::TcpListener;
use tokio_modbus::server::tcp::{accept_tcp_connection, Server};
use tracing::{error, info};
use wireplant_core::memory::{SessionCounter, SharedProcessImage};

use crate::handler::ModbusHandler;

/// Modbus TCP listener and client connection supervisor.
#[derive(Debug)]
pub struct ModbusServer {
    default_unit_id: u8,
    allowed_unit_ids: Vec<u8>,
    max_connections: usize,
    sessions: SessionCounter,
    shared_memory: SharedProcessImage,
}

impl ModbusServer {
    pub fn new(
        default_unit_id: u8,
        allowed_unit_ids: Vec<u8>,
        max_connections: usize,
        sessions: SessionCounter,
        shared_memory: SharedProcessImage,
    ) -> Self {
        Self {
            default_unit_id,
            allowed_unit_ids,
            max_connections,
            sessions,
            shared_memory,
        }
    }

    /// Serves an already-bound listener until the broadcast fires.
    ///
    /// Binding is the caller's job so that a port already in use is reported at
    /// startup rather than by a task that dies quietly later.
    pub async fn serve(
        self,
        listener: TcpListener,
        mut cancel_rx: tokio::sync::broadcast::Receiver<()>,
    ) {
        let local_addr = listener
            .local_addr()
            .map(|addr| addr.to_string())
            .unwrap_or_else(|_| "<unknown>".to_string());
        info!(
            "Modbus TCP server listening on {local_addr} (Unit ID: {})",
            self.default_unit_id
        );

        let server = Server::new(listener);
        let sessions = self.sessions.clone();
        let max_connections = self.max_connections;
        let shared_memory = self.shared_memory.clone();
        let allowed_unit_ids = self.allowed_unit_ids.clone();

        // Every connection gets its own handler, which carries the slot it
        // claimed; the accept loop is the one place a client can be refused.
        let new_service = move |_socket_addr: SocketAddr| {
            let Some(slot) = sessions.try_open(max_connections) else {
                return Ok(None);
            };

            Ok(Some(ModbusHandler::new(
                shared_memory.clone(),
                allowed_unit_ids.clone(),
                Some(slot),
            )))
        };

        let on_connected = move |stream, socket_addr| {
            let service_fn = new_service.clone();

            async move {
                let connection = accept_tcp_connection(stream, socket_addr, service_fn);
                if matches!(connection, Ok(None)) {
                    info!(
                        "Rejecting Modbus connection from {socket_addr}: \
                         {max_connections} of {max_connections} slots in use"
                    );
                }
                connection
            }
        };

        let on_process_error = |err| {
            error!("Modbus server process error: {err}");
        };

        tokio::select! {
            _ = cancel_rx.recv() => {
                info!("Modbus TCP server received shutdown signal");
            }
            res = server.serve(&on_connected, on_process_error) => {
                if let Err(e) = res {
                    error!("Modbus server terminated with error: {e}");
                }
            }
        }
    }
}

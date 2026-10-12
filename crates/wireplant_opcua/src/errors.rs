use thiserror::Error;

/// Error types produced by the WirePlant OPC-UA subsystem.
#[derive(Debug, Error)]
pub enum OpcUaError {
    #[error("Invalid bind address endpoint '{addr}': {reason}")]
    InvalidEndpoint { addr: String, reason: String },

    #[error("Failed to acquire OPC-UA SimpleNodeManager from server handle")]
    NodeManagerMissing,

    #[error("OPC-UA server build error: {0}")]
    ServerBuild(String),

    #[error("IO error preparing PKI storage: {0}")]
    PkiIo(#[from] std::io::Error),
}

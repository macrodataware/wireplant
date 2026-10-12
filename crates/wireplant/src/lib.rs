//! # WirePlant
//!
//! **Industrial OT Protocol Simulator & Telemetry Digital Twin**
//!
//! WirePlant simulates an operational technology (OT) facility in software. It boots
//! simulated physical processes (such as tanks with PID loops and thermal dynamics),
//! steps them against a deterministic virtual clock, and exposes real-time telemetry over
//! industry-standard protocols including:
//!
//! - **Modbus TCP & RTU** (Holding registers, Input registers, Coils, Discrete inputs)
//! - **OPC-UA** (Server with typed variable nodes and write callbacks)
//! - **MQTT** (Configurable JSON telemetry payloads and command subscriptions)
//! - **HTTP REST API** (Tag inspection, manual value overrides, fault injection, alarm acknowledgement)
//! - **Terminal UI (TUI)** (Real-time monitoring, live overrides, alarm management, log stream)
//! - **Embedded SQLite Historian** (Telemetry recording, queries, and CSV/JSON export)
//!
//! ## Library Usage
//!
//! While typically run via the `wireplant` binary, this crate can also be used as a library to
//! validate scenarios, query recorded historian files, or orchestrate simulated facilities
//! programmatically.
//!
//! ```rust,no_run
//! use wireplant::args::CliArgs;
//! use wireplant::orchestrator::Orchestrator;
//! use clap::Parser;
//!
//! # async fn run() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
//! let args = CliArgs::parse_from(["wireplant", "--headless"]);
//! Orchestrator::run(args, None).await?;
//! # Ok(())
//! # }
//! ```

pub mod args;
pub mod export;
pub mod orchestrator;
pub mod shutdown;
pub mod validate;

pub use args::{CliArgs, CliCommand};
pub use export::{run_export, ExportOptions};
pub use orchestrator::Orchestrator;
pub use shutdown::wait_for_shutdown_signal;
pub use validate::run_validation;

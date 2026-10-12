//! # WirePlant TUI (`wireplant_tui`)
//!
//! Interactive terminal dashboard built with `ratatui` and `crossterm` providing live
//! telemetry sparklines, register memory inspection, fault studio injection, and alarm acknowledgment.

pub mod app;
pub mod errors;
pub mod tracing_bridge;
pub mod ui;

pub use app::{ActiveTab, AppMode, LogEntry, LogLevel, TuiApp};
pub use errors::TuiMemoryError;
pub use tracing_bridge::TuiTracingLayer;
pub use ui::draw_ui;

//! Interactive Ratatui terminal dashboard for WirePlant.

pub mod app;
pub mod tracing_bridge;
pub mod ui;

pub use app::{ActiveTab, AppMode, LogEntry, LogLevel, TuiApp};
pub use tracing_bridge::TuiTracingLayer;
pub use ui::draw_ui;

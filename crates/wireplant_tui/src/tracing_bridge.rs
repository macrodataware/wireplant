//! Tracing subscriber layer for the WirePlant terminal dashboard.
//!
//! Terminal raw mode disables standard stdout line buffering; writing to stdout directly
//! tears the screen layout. This layer intercepts tracing events across the workspace and
//! delivers them to the TUI event log over an unbounded channel.

use std::time::Instant;
use tokio::sync::mpsc;
use tracing::field::Visit;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::Context;
use tracing_subscriber::Layer;

use crate::app::{LogEntry, LogLevel};

#[derive(Default)]
struct MessageVisitor {
    message: String,
    fields: String,
}

impl MessageVisitor {
    fn push_field(&mut self, name: &str, value: &str) {
        if self.fields.is_empty() {
            self.fields = format!("{name}={value}");
        } else {
            self.fields.push_str(&format!(", {name}={value}"));
        }
    }

    fn finish(self) -> String {
        match (self.message.is_empty(), self.fields.is_empty()) {
            (true, true) => String::new(),
            (false, true) => self.message,
            (true, false) => self.fields,
            (false, false) => format!("{} ({})", self.message, self.fields),
        }
    }
}

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = format!("{value:?}");
        } else {
            self.push_field(field.name(), &format!("{value:?}"));
        }
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.message = value.to_string();
        } else {
            self.push_field(field.name(), value);
        }
    }
}

/// Tracing layer capturing events and feeding the TUI event stream.
#[derive(Debug)]
pub struct TuiTracingLayer {
    sender: mpsc::UnboundedSender<LogEntry>,
    start_time: Instant,
}

impl TuiTracingLayer {
    pub fn new(sender: mpsc::UnboundedSender<LogEntry>, start_time: Instant) -> Self {
        Self { sender, start_time }
    }
}

impl<S: Subscriber> Layer<S> for TuiTracingLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let metadata = event.metadata();
        let level = match *metadata.level() {
            Level::ERROR => LogLevel::Error,
            Level::WARN => LogLevel::Warn,
            _ => LogLevel::Info,
        };

        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);
        let message_text = visitor.finish();

        let target = metadata.target();
        let target_short = target.strip_prefix("wireplant_").unwrap_or(target);

        let formatted = if !message_text.is_empty() {
            format!("[{target_short}] {message_text}")
        } else {
            format!("[{target_short}] event")
        };

        let entry = LogEntry {
            timestamp_secs: self.start_time.elapsed().as_secs(),
            message: formatted,
            level,
        };

        let _ = self.sender.send(entry);
    }
}

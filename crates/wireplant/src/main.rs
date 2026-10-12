use clap::Parser;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use wireplant::args::{CliArgs, CliCommand};
use wireplant::export::{self, ExportOptions};
use wireplant::orchestrator::Orchestrator;
use wireplant::validate;

/// Sets up the process-wide tracing subscriber before starting the runtime.
///
/// When running with the TUI, an unbounded channel is connected to `wireplant_tui::TuiTracingLayer`
/// while omitting `fmt::layer()` so raw terminal mode remains uncorrupted by ANSI sequences.
/// The receiving end is handed to the orchestrator to populate the TUI event log tab.
fn init_tracing(
    wants_tui: bool,
) -> Option<tokio::sync::mpsc::UnboundedReceiver<wireplant_tui::LogEntry>> {
    let filter =
        tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into());

    if wants_tui {
        let (log_tx, log_rx) = tokio::sync::mpsc::unbounded_channel();
        let app_start = std::time::Instant::now();
        let tui_layer = wireplant_tui::TuiTracingLayer::new(log_tx, app_start);

        tracing_subscriber::registry()
            .with(filter)
            .with(tui_layer)
            .init();

        Some(log_rx)
    } else {
        tracing_subscriber::registry()
            .with(filter)
            .with(tracing_subscriber::fmt::layer())
            .init();

        None
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let args = CliArgs::parse();

    if let Some(target) = args.validation_target() {
        return match validate::run_validation(target) {
            Ok(()) => Ok(()),
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        };
    }

    if let Some(CliCommand::ExportHistory {
        db,
        tag,
        format,
        output,
        from,
        to,
        limit,
    }) = args.command.clone()
    {
        let opts = ExportOptions {
            db_path: db,
            tag_id: tag,
            format,
            output_file: output,
            from_ms: from,
            to_ms: to,
            limit,
        };
        return match export::run_export(opts) {
            Ok(()) => Ok(()),
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        };
    }

    let log_rx = init_tracing(args.wants_tui());

    Orchestrator::run(args, log_rx).await
}

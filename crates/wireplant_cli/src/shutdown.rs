use tokio::signal;
use tracing::info;

/// Waits for an OS termination signal (Ctrl+C or SIGTERM).
pub(crate) async fn wait_for_shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("Failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("Failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {
            info!("Received Ctrl+C (SIGINT)");
        }
        _ = terminate => {
            info!("Received SIGTERM");
        }
    }
}

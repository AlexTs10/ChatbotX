//! `chatbotx-server`: the single ChatbotX binary.

mod config;

use std::process::ExitCode;

use config::Config;
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

fn main() -> ExitCode {
    let config = match Config::from_env() {
        Ok(config) => config,
        Err(err) => {
            eprintln!("chatbotx-server: {err}");
            return ExitCode::FAILURE;
        }
    };
    init_tracing(&config.log_level);

    // A small, fixed number of worker threads keeps the idle memory low.
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(config.worker_threads)
        .thread_name("chatbotx-worker")
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(err) => {
            tracing::error!(error = %err, "cannot start the tokio runtime");
            return ExitCode::FAILURE;
        }
    };

    match runtime.block_on(serve(config)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            tracing::error!(error = %err, "server stopped with an error");
            ExitCode::FAILURE
        }
    }
}

fn init_tracing(log_level: &str) {
    let filter = EnvFilter::try_new(log_level).unwrap_or_else(|err| {
        eprintln!("chatbotx-server: LOG_LEVEL={log_level:?} is not valid ({err}); using info");
        EnvFilter::new("info")
    });
    tracing_subscriber::fmt()
        .compact()
        .with_env_filter(filter)
        .init();
}

async fn serve(config: Config) -> std::io::Result<()> {
    let listener = TcpListener::bind((config.host.as_str(), config.port)).await?;
    tracing::info!(
        addr = %listener.local_addr()?,
        data_dir = %config.data_dir.display(),
        worker_threads = config.worker_threads,
        "listening"
    );
    axum::serve(listener, chatbotx_api::router())
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    tracing::info!("shutdown complete");
    Ok(())
}

/// Resolve on SIGINT (Ctrl-C) or SIGTERM.
async fn shutdown_signal() {
    let interrupt = async {
        if let Err(err) = tokio::signal::ctrl_c().await {
            tracing::error!(error = %err, "cannot listen for SIGINT");
            std::future::pending::<()>().await;
        }
    };

    #[cfg(unix)]
    let terminate = async {
        use tokio::signal::unix::{SignalKind, signal};
        match signal(SignalKind::terminate()) {
            Ok(mut stream) => {
                stream.recv().await;
            }
            Err(err) => {
                tracing::error!(error = %err, "cannot listen for SIGTERM");
                std::future::pending::<()>().await;
            }
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = interrupt => tracing::info!("SIGINT received, shutting down"),
        () = terminate => tracing::info!("SIGTERM received, shutting down"),
    }
}

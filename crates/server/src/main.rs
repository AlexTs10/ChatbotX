//! `chatbotx-server`: the single ChatbotX binary.

mod config;

use std::process::ExitCode;
use std::sync::Arc;

use chatbotx_db::Db;
use chatbotx_kv::{Clock, Kv, KvConfig, SystemClock};
use chatbotx_rt::{KvPresence, NoRevokeRetry, Realtime, RtConfig};
use config::Config;
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

/// The SQLite file of the realtime revoke markers (`kv_entry`), under `DATA_DIR`.
const DB_FILE: &str = "chatbotx.db";

fn main() -> ExitCode {
    let config = match Config::from_env() {
        Ok(config) => config,
        Err(err) => {
            eprintln!("chatbotx-server: {err}");
            return ExitCode::FAILURE;
        }
    };
    // The realtime gateway is on when `REALTIME_BROADCAST_SECRET` is set.
    let realtime = match realtime_config() {
        Ok(realtime) => realtime,
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

    match runtime.block_on(serve(config, realtime)) {
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

/// `Some` when `REALTIME_BROADCAST_SECRET` is set (then it must be valid).
fn realtime_config() -> Result<Option<RtConfig>, chatbotx_rt::RtConfigError> {
    let secret_set =
        std::env::var("REALTIME_BROADCAST_SECRET").is_ok_and(|secret| !secret.is_empty());
    if !secret_set {
        return Ok(None);
    }
    RtConfig::from_env().map(Some)
}

/// Opens the database, the kv stores and the realtime hub.
async fn start_realtime(config: &Config, rt_config: RtConfig) -> std::io::Result<Realtime> {
    std::fs::create_dir_all(&config.data_dir)?;
    let db = Db::open(config.data_dir.join(DB_FILE)).map_err(std::io::Error::other)?;
    db.migrate().await.map_err(std::io::Error::other)?;
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    let kv = Kv::start(Arc::new(db), Arc::clone(&clock), KvConfig::default());
    Ok(Realtime::start(
        Arc::clone(&kv),
        clock,
        rt_config,
        Arc::new(KvPresence::new(kv)),
        Arc::new(NoRevokeRetry),
    ))
}

async fn serve(config: Config, rt_config: Option<RtConfig>) -> std::io::Result<()> {
    let realtime = match rt_config {
        Some(rt_config) => Some(start_realtime(&config, rt_config).await?),
        None => {
            tracing::info!("realtime gateway off: REALTIME_BROADCAST_SECRET is not set");
            None
        }
    };
    let mut app = chatbotx_api::router();
    if let Some(realtime) = &realtime {
        app = app.merge(chatbotx_rt::router(realtime.clone()));
    }
    let listener = TcpListener::bind((config.host.as_str(), config.port)).await?;
    tracing::info!(
        addr = %listener.local_addr()?,
        data_dir = %config.data_dir.display(),
        worker_threads = config.worker_threads,
        "listening"
    );
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal(realtime))
        .await?;
    tracing::info!("shutdown complete");
    Ok(())
}

/// Resolve on SIGINT (Ctrl-C) or SIGTERM, after the realtime drain
/// (producers flushed, sockets closed 4003 with `retryAfter`).
async fn shutdown_signal(realtime: Option<Realtime>) {
    wait_for_signal().await;
    if let Some(realtime) = realtime {
        realtime.shutdown().await;
    }
}

async fn wait_for_signal() {
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

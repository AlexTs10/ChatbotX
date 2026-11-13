//! `chatbotx-server`: the single ChatbotX binary.

mod config;

use std::process::ExitCode;
use std::sync::Arc;

use chatbotx_api::auth::LiveAuthDeps;
use chatbotx_api::mail::{Mailer, NoMailer, SmtpMailer};
use chatbotx_api::session::{AuthSettings, AuthState, TrustedOriginSettings};
use chatbotx_db::Db;
use chatbotx_kv::{Clock, Kv, KvConfig, SystemClock};
use chatbotx_rt::{KvPresence, NoRevokeRetry, Realtime, RtConfig};
use config::Config;
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

/// The SQLite database (app tables and the kv rows), under `DATA_DIR`.
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

/// The database and the kv stores every part of the server shares.
struct Shared {
    db: Arc<Db>,
    kv: Arc<Kv>,
    clock: Arc<dyn Clock>,
}

/// Opens (and migrates) the database under `DATA_DIR` and starts the kv stores.
async fn open_shared(config: &Config) -> std::io::Result<Shared> {
    std::fs::create_dir_all(&config.data_dir)?;
    let db = Db::open(config.data_dir.join(DB_FILE)).map_err(std::io::Error::other)?;
    db.migrate().await.map_err(std::io::Error::other)?;
    let db = Arc::new(db);
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    let kv = Kv::start(Arc::clone(&db), Arc::clone(&clock), KvConfig::default());
    Ok(Shared { db, kv, clock })
}

/// Starts the realtime hub on the shared kv stores.
fn start_realtime(shared: &Shared, rt_config: RtConfig) -> Realtime {
    Realtime::start(
        Arc::clone(&shared.kv),
        Arc::clone(&shared.clock),
        rt_config,
        Arc::new(KvPresence::new(Arc::clone(&shared.kv))),
        Arc::new(NoRevokeRetry),
    )
}

fn env_or(name: &str, fallback: &str) -> String {
    std::env::var(name)
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| fallback.to_string())
}

/// `BETTER_AUTH_SECRET`, `BETTER_AUTH_URL`, `AUTH_DISABLE_SIGNUP`, the trusted origins
/// (`NEXT_PUBLIC_BROKER_URL`, `NEXT_PUBLIC_BUILDER_URL`, `NEXT_PHASE`, `BETTER_AUTH_TRUSTED_ORIGINS`).
fn auth_settings() -> AuthSettings {
    AuthSettings {
        secret: env_or("BETTER_AUTH_SECRET", ""),
        base_url: env_or("BETTER_AUTH_URL", "http://localhost:3000")
            .trim_end_matches('/')
            .to_string(),
        sign_up_disabled: env_or("AUTH_DISABLE_SIGNUP", "false") == "true",
        trusted_origins: TrustedOriginSettings {
            broker_url: std::env::var("NEXT_PUBLIC_BROKER_URL")
                .ok()
                .filter(|url| !url.is_empty()),
            builder_url: env_or("NEXT_PUBLIC_BUILDER_URL", ""),
            next_phase: std::env::var("NEXT_PHASE").ok(),
        },
        extra_trusted_origins: std::env::var("BETTER_AUTH_TRUSTED_ORIGINS").ok(),
    }
}

/// `SMTP_SERVER` + `SMTP_FROM` (plain SMTP), else mail is dropped.
fn mailer() -> Arc<dyn Mailer> {
    let from = env_or("SMTP_FROM", "no-reply@localhost");
    match SmtpMailer::from_url(&env_or("SMTP_SERVER", ""), &from) {
        Some(mailer) => Arc::new(mailer),
        None => Arc::new(NoMailer),
    }
}

fn now_ms() -> i64 {
    SystemClock.now_ms()
}

/// `isCloud()`: `NEXT_PUBLIC_EDITION=cloud`.
fn is_cloud() -> bool {
    std::env::var("NEXT_PUBLIC_EDITION").is_ok_and(|edition| edition == "cloud")
}

async fn serve(config: Config, rt_config: Option<RtConfig>) -> std::io::Result<()> {
    let shared = open_shared(&config).await?;
    let realtime = match rt_config {
        Some(rt_config) => Some(start_realtime(&shared, rt_config)),
        None => {
            tracing::info!("realtime gateway off: REALTIME_BROADCAST_SECRET is not set");
            None
        }
    };
    let auth = Arc::new(LiveAuthDeps {
        db: Arc::clone(&shared.db),
        kv: Arc::clone(&shared.kv),
        clock: Arc::clone(&shared.clock),
        is_cloud: is_cloud(),
    });
    let sessions = Arc::new(AuthState {
        db: Arc::clone(&shared.db),
        settings: auth_settings(),
        mailer: mailer(),
        now_ms,
    });
    let mut app = chatbotx_api::router()
        .merge(chatbotx_api::rpc::rpc_router(Arc::clone(&sessions)))
        .merge(chatbotx_api::session::auth_router(sessions))
        .merge(chatbotx_api::routes::public_api_router_with(
            auth,
            Arc::new(chatbotx_api::handlers::ApiState {
                db: Arc::clone(&shared.db),
                app_url: env_or("NEXT_PUBLIC_BUILDER_URL", "http://localhost:3000")
                    .trim_end_matches('/')
                    .to_string(),
            }),
        ));
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

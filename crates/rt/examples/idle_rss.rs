//! Idle cost of the realtime gateway (spec 3.7): RSS (`VmRSS`, `VmHWM`)
//! and wake-ups per second with 0 and 1 open sockets.
//!
//! Production start (`Kv::start` on SQLite, `Realtime::start`, the axum
//! router) on a 2-worker runtime, as the server binary. The client runs on
//! its own OS thread (`client`), which is left out of the wake-up count.
//! A wake-up = one context switch of a process thread
//! (`/proc/self/task/*/status`, voluntary + non-voluntary). Linux only.
//!
//! Run: `cargo run --release -p chatbotx-rt --example idle_rss`
//! (`RT_MEASURE_SECS`, default 30: the idle window; the one-socket window
//! is twice as long, so it holds two heartbeats).

use std::sync::Arc;
use std::time::Duration;

use chatbotx_db::Db;
use chatbotx_kv::{Clock, Kv, KvConfig, SystemClock};
use chatbotx_rt::token::{ChatScope, MemberConnect, sign_member_connect_token};
use chatbotx_rt::{KvPresence, NoRevokeRetry, Realtime, RtConfig, router};
use futures_util::StreamExt;

const SECRET: &str = "idle-rss-secret-idle-rss-secret-1";

/// `(VmRSS, VmHWM)` in KiB.
fn memory_kib() -> Option<(u64, u64)> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let field = |name: &str| {
        status
            .lines()
            .find(|line| line.starts_with(name))
            .and_then(|line| line.split_whitespace().nth(1))
            .and_then(|value| value.parse().ok())
    };
    Some((field("VmRSS:")?, field("VmHWM:")?))
}

/// Context switches of every process thread except `client`.
fn context_switches() -> Option<u64> {
    let mut total = 0;
    for task in std::fs::read_dir("/proc/self/task").ok()? {
        let path = task.ok()?.path();
        let name = std::fs::read_to_string(path.join("comm")).unwrap_or_default();
        if name.trim() == "client" {
            continue;
        }
        let status = std::fs::read_to_string(path.join("status")).unwrap_or_default();
        for line in status.lines() {
            if line.starts_with("voluntary_ctxt_switches:")
                || line.starts_with("nonvoluntary_ctxt_switches:")
            {
                total += line
                    .split_whitespace()
                    .nth(1)
                    .and_then(|value| value.parse::<u64>().ok())
                    .unwrap_or(0);
            }
        }
    }
    Some(total)
}

fn measure(label: &str, window: Duration) {
    let before = context_switches();
    std::thread::sleep(window);
    let after = context_switches();
    let wakeups = match (before, after) {
        (Some(before), Some(after)) => {
            format!("{:.3}", (after - before) as f64 / window.as_secs_f64())
        }
        _ => "n/a".to_owned(),
    };
    let memory = memory_kib().map_or_else(
        || "n/a".to_owned(),
        |(rss, hwm)| format!("VmRSS {rss} KiB, VmHWM {hwm} KiB"),
    );
    println!(
        "{label}: {memory}, wake-ups/s {wakeups} (window {} s)",
        window.as_secs()
    );
}

fn main() {
    let secs: u64 = std::env::var("RT_MEASURE_SECS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(30);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_name("chatbotx-worker")
        .enable_all()
        .build()
        .expect("runtime");
    let port = runtime.block_on(async {
        let db = Db::open_in_memory().expect("db");
        db.migrate().await.expect("migrate");
        let clock: Arc<dyn Clock> = Arc::new(SystemClock);
        let kv = Kv::start(Arc::new(db), Arc::clone(&clock), KvConfig::default());
        let mut cfg = RtConfig::new(SECRET);
        cfg.shutdown_drain_ms = 0;
        let rt = Realtime::start(
            Arc::clone(&kv),
            clock,
            cfg,
            Arc::new(KvPresence::new(kv)),
            Arc::new(NoRevokeRetry),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let port = listener.local_addr().expect("addr").port();
        tokio::spawn(async move {
            axum::serve(listener, router(rt)).await.expect("serve");
        });
        port
    });
    std::thread::sleep(Duration::from_secs(2));
    measure("0 sockets", Duration::from_secs(secs));

    let token = sign_member_connect_token(
        &MemberConnect {
            workspace_id: "ws-1".to_owned(),
            user_id: "user-1".to_owned(),
            chat_scope: ChatScope::All,
            team_ids: Vec::new(),
            issued_at_ms: SystemClock.now_ms(),
        },
        SECRET,
    )
    .expect("token");
    let url = format!("ws://127.0.0.1:{port}/rt/workspaces/ws-1?token={token}");
    let (opened_tx, opened_rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("client".to_owned())
        .spawn(move || {
            let client_runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("client runtime");
            client_runtime.block_on(async move {
                let (mut socket, _) = tokio_tungstenite::connect_async(url)
                    .await
                    .expect("connect");
                let _ = opened_tx.send(());
                // Reading answers the server pings (as a browser does).
                while let Some(Ok(_)) = socket.next().await {}
            });
        })
        .expect("client thread");
    opened_rx.recv().expect("socket open");
    std::thread::sleep(Duration::from_secs(2));
    measure("1 socket", Duration::from_secs(secs * 2));
    drop(runtime);
}

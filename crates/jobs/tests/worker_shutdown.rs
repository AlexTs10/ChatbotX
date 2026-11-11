//! Port of `apps/worker/__tests__/worker-shutdown.test.ts` (spec 2.13, 5.3
//! "worker-shutdown"): the shutdown coordinator. `process.exit` is the
//! injected exit recorder; signals are manual `request` calls; fake timers
//! are tokio's paused clock.

#[path = "support/shutdown.rs"]
mod shutdown_support;
mod support;

use std::time::Duration;

use chatbotx_jobs::shutdown::{CrashKind, Shutdown, ShutdownReason, StartFn};
use shutdown_support::{
    ExitRecorder, OrderLog, err, failing_bootstrap, failing_closer, gated_bootstrap,
    hanging_closer, ok_closer, recording_bootstrap, settle, shutdown_with, wait_until, within,
};
use support::RecordingLog;
use tokio::sync::oneshot;
use tracing::Level;

fn ok_start(log: &OrderLog) -> StartFn {
    let log = log.clone();
    Box::new(move || {
        log.push("start");
        Box::pin(async { Ok(()) })
    })
}

/// Port of: apps/worker/__tests__/worker-shutdown.test.ts:73 "worker shutdown coordinator > closes every registered worker and exits 0"
#[tokio::test(start_paused = true)]
async fn closes_every_registered_worker_and_exits_0() {
    let exit = ExitRecorder::new();
    let shutdown = shutdown_with(&exit);
    let log = OrderLog::new();
    shutdown.on_shutdown("chat", ok_closer(&log, "chat"));
    shutdown.on_shutdown("low", ok_closer(&log, "low"));

    within(shutdown.request(ShutdownReason::Sigterm, None)).await;

    assert_eq!(log.count("chat"), 1);
    assert_eq!(log.count("low"), 1);
    assert_eq!(exit.codes(), vec![0]);
}

/// Port of: apps/worker/__tests__/worker-shutdown.test.ts:86 "worker shutdown coordinator > still closes the other workers and exits 1 when one close fails"
#[tokio::test(start_paused = true)]
async fn still_closes_the_other_workers_and_exits_1_when_one_close() {
    let records = RecordingLog::install();
    let exit = ExitRecorder::new();
    let shutdown = shutdown_with(&exit);
    let log = OrderLog::new();
    shutdown.on_shutdown("chat", failing_closer("boom"));
    shutdown.on_shutdown("low", ok_closer(&log, "low"));

    within(shutdown.request(ShutdownReason::Sigterm, None)).await;

    assert_eq!(log.count("low"), 1);
    let error = records
        .find("Error during worker shutdown")
        .expect("error logged");
    assert_eq!(error.level, Level::ERROR);
    assert_eq!(error.field("worker"), Some("chat"));
    assert_eq!(exit.codes(), vec![1]);
}

/// Port of: apps/worker/__tests__/worker-shutdown.test.ts:101 "worker shutdown coordinator > runs finalizers only after every worker closer settles"
#[tokio::test(start_paused = true)]
async fn runs_finalizers_only_after_every_worker_closer_settles() {
    let exit = ExitRecorder::new();
    let shutdown = shutdown_with(&exit);
    let events = OrderLog::new();
    let (release_chat, chat_closed) = oneshot::channel::<()>();

    let ev = events.clone();
    shutdown.on_shutdown(
        "chat",
        Box::new(move || {
            ev.push("chat:closing");
            Box::pin(async move {
                let _ = chat_closed.await;
                ev.push("chat:closed");
                Ok(())
            })
        }),
    );
    shutdown.on_shutdown("low", ok_closer(&events, "low:closed"));
    shutdown.on_shutdown_finalize("realtime", ok_closer(&events, "realtime:flushed"));

    let shutdown_done = tokio::spawn(shutdown.request(ShutdownReason::Sigterm, None));
    wait_until("chat:closing", || events.count("chat:closing") == 1).await;
    assert_eq!(events.count("realtime:flushed"), 0);

    release_chat.send(()).expect("chat closer waits");
    within(shutdown_done).await.expect("shutdown task");

    assert_eq!(
        events.events(),
        vec![
            "chat:closing",
            "low:closed",
            "chat:closed",
            "realtime:flushed"
        ]
    );
}

/// Port of: apps/worker/__tests__/worker-shutdown.test.ts:134 "worker shutdown coordinator > keeps the first finalizer registered for a name"
#[tokio::test(start_paused = true)]
async fn keeps_the_first_finalizer_registered_for_a_name() {
    let exit = ExitRecorder::new();
    let shutdown = shutdown_with(&exit);
    let log = OrderLog::new();
    shutdown.on_shutdown("chat", ok_closer(&log, "chat"));
    shutdown.on_shutdown_finalize("realtime", ok_closer(&log, "first"));
    shutdown.on_shutdown_finalize("realtime", ok_closer(&log, "duplicate"));

    within(shutdown.request(ShutdownReason::Sigterm, None)).await;

    assert_eq!(log.count("first"), 1);
    assert_eq!(log.count("duplicate"), 0);
}

/// Port of: apps/worker/__tests__/worker-shutdown.test.ts:147 "worker shutdown coordinator > ignores and logs closers registered after shutdown begins"
#[tokio::test(start_paused = true)]
async fn ignores_and_logs_closers_registered_after_shutdown_begins() {
    let records = RecordingLog::install();
    let exit = ExitRecorder::new();
    let shutdown = shutdown_with(&exit);
    let (release_chat, chat_closed) = oneshot::channel::<()>();
    shutdown.on_shutdown(
        "chat",
        Box::new(move || {
            Box::pin(async move {
                let _ = chat_closed.await;
                Ok(())
            })
        }),
    );

    let shutdown_done = tokio::spawn(shutdown.request(ShutdownReason::Sigterm, None));
    wait_until("Shutting down", || {
        records.has(Level::INFO, "Shutting down")
    })
    .await;
    let log = OrderLog::new();
    shutdown.on_shutdown("late", ok_closer(&log, "late"));
    release_chat.send(()).expect("chat closer waits");
    within(shutdown_done).await.expect("shutdown task");

    assert_eq!(log.count("late"), 0);
    let warn = records
        .find("Ignoring worker shutdown closer registered after shutdown began")
        .expect("warn logged");
    assert_eq!(warn.level, Level::WARN);
    assert_eq!(warn.field("worker"), Some("late"));
}

/// Port of: apps/worker/__tests__/worker-shutdown.test.ts:171 "worker shutdown coordinator > does not arm a shutdown deadline when timeout is unset"
#[tokio::test(start_paused = true)]
async fn does_not_arm_a_shutdown_deadline_when_timeout_is_unset() {
    let exit = ExitRecorder::new();
    let shutdown = shutdown_with(&exit);
    let log = OrderLog::new();
    shutdown.on_shutdown("chat", ok_closer(&log, "chat"));

    within(shutdown.request(ShutdownReason::Sigterm, None)).await;

    assert!(!shutdown.deadline_armed());
}

/// Port of: apps/worker/__tests__/worker-shutdown.test.ts:180 "worker shutdown coordinator > exits 1 and names the stuck workers when draining exceeds the deadline"
#[tokio::test(start_paused = true)]
async fn exits_1_and_names_the_stuck_workers_when_draining_exceeds() {
    let records = RecordingLog::install();
    let exit = ExitRecorder::new();
    let shutdown = Shutdown::new(Some(1000), exit.exit_fn(), shutdown_support::ok_bootstrap());
    let log = OrderLog::new();
    shutdown.on_shutdown("heavy", hanging_closer());
    shutdown.on_shutdown("low", ok_closer(&log, "low"));

    let _drain = tokio::spawn(shutdown.request(ShutdownReason::Sigterm, None));
    tokio::time::sleep(Duration::from_millis(1000)).await;
    settle().await;

    let error = records
        .find("Shutdown timed out with workers still draining")
        .expect("error logged");
    assert_eq!(error.level, Level::ERROR);
    assert_eq!(error.field("pending"), Some(r#"["heavy"]"#));
    assert_eq!(error.field("timeout_ms"), Some("1000"));
    assert_eq!(exit.codes(), vec![1]);
}

/// Port of: apps/worker/__tests__/worker-shutdown.test.ts:196 "worker shutdown coordinator > bounds a fatal-error drain at 30s even without a configured timeout"
#[tokio::test(start_paused = true)]
async fn bounds_a_fatal_error_drain_at_30s_even_without_a_configured() {
    let exit = ExitRecorder::new();
    let shutdown = shutdown_with(&exit);
    shutdown.on_shutdown("heavy", hanging_closer());

    let _drain = tokio::spawn(shutdown.request(ShutdownReason::UncaughtException, Some(1)));
    tokio::time::sleep(Duration::from_millis(29_999)).await;
    settle().await;
    assert!(exit.codes().is_empty());
    tokio::time::sleep(Duration::from_millis(1)).await;
    settle().await;

    assert_eq!(exit.codes(), vec![1]);
}

/// Port of: apps/worker/__tests__/worker-shutdown.test.ts:208 "worker shutdown coordinator > never arms a deadline for a graceful SIGTERM drain"
#[tokio::test(start_paused = true)]
async fn never_arms_a_deadline_for_a_graceful_sigterm_drain() {
    let exit = ExitRecorder::new();
    let shutdown = shutdown_with(&exit);
    shutdown.on_shutdown("heavy", hanging_closer());

    let _drain = tokio::spawn(shutdown.request(ShutdownReason::Sigterm, None));
    tokio::time::sleep(Duration::from_millis(10 * 60_000)).await;
    settle().await;

    assert!(exit.codes().is_empty());
}

/// Port of: apps/worker/__tests__/worker-shutdown.test.ts:218 "worker shutdown coordinator > ignores a second shutdown request while one is in progress"
#[tokio::test(start_paused = true)]
async fn ignores_a_second_shutdown_request_while_one_is_in_progress() {
    let exit = ExitRecorder::new();
    let shutdown = shutdown_with(&exit);
    let log = OrderLog::new();
    shutdown.on_shutdown("chat", ok_closer(&log, "chat"));

    within(async {
        tokio::join!(
            shutdown.request(ShutdownReason::Sigterm, None),
            shutdown.request(ShutdownReason::Sigint, None),
        )
    })
    .await;

    assert_eq!(log.count("chat"), 1);
    assert_eq!(exit.codes().len(), 1);
}

/// Port of: apps/worker/__tests__/worker-shutdown.test.ts:231 "worker shutdown coordinator > a worker that fails to start drains the others and exits 1"
#[tokio::test(start_paused = true)]
async fn a_worker_that_fails_to_start_drains_the_others_and_exits_1() {
    let records = RecordingLog::install();
    let exit = ExitRecorder::new();
    let shutdown = shutdown_with(&exit);
    let log = OrderLog::new();
    shutdown.on_shutdown("chat", ok_closer(&log, "chat"));

    let _run = shutdown.run_worker(
        "webhook",
        Box::new(|| Box::pin(async { Err(err("no redis")) })),
    );
    wait_until("exit(1)", || exit.codes().contains(&1)).await;

    let error = records
        .find("Failed to start worker")
        .expect("error logged");
    assert_eq!(error.level, Level::ERROR);
    assert_eq!(error.field("worker"), Some("webhook"));
    assert_eq!(log.count("chat"), 1);
}

/// Port of: apps/worker/__tests__/worker-shutdown.test.ts:245 "worker shutdown coordinator > runWorker bootstraps before starting the worker"
#[tokio::test(start_paused = true)]
async fn runworker_bootstraps_before_starting_the_worker() {
    let exit = ExitRecorder::new();
    let order = OrderLog::new();
    let shutdown = Shutdown::new(None, exit.exit_fn(), recording_bootstrap(order.clone()));

    let _run = shutdown.run_worker("chat", ok_start(&order));
    wait_until("start", || order.count("start") == 1).await;

    assert_eq!(order.events(), vec!["bootstrap", "start"]);
}

/// Port of: apps/worker/__tests__/worker-shutdown.test.ts:262 "worker shutdown coordinator > a bootstrap failure skips start and exits 1"
#[tokio::test(start_paused = true)]
async fn a_bootstrap_failure_skips_start_and_exits_1() {
    let records = RecordingLog::install();
    let exit = ExitRecorder::new();
    let shutdown = Shutdown::new(None, exit.exit_fn(), failing_bootstrap("license invalid"));
    let order = OrderLog::new();

    let _run = shutdown.run_worker("chat", ok_start(&order));
    wait_until("exit(1)", || exit.codes().contains(&1)).await;

    assert_eq!(order.count("start"), 0);
    let error = records
        .find("Failed to start worker")
        .expect("error logged");
    assert_eq!(error.level, Level::ERROR);
    assert_eq!(error.field("worker"), Some("chat"));
}

/// Port of: apps/worker/__tests__/worker-shutdown.test.ts:276 "worker shutdown coordinator > skips start when shutdown began while bootstrap was pending"
#[tokio::test(start_paused = true)]
async fn skips_start_when_shutdown_began_while_bootstrap_was_pending() {
    let records = RecordingLog::install();
    let exit = ExitRecorder::new();
    let (bootstrap, finish_bootstrap) = gated_bootstrap();
    let shutdown = Shutdown::new(None, exit.exit_fn(), bootstrap);
    let order = OrderLog::new();

    let _run = shutdown.run_worker("chat", ok_start(&order));
    within(shutdown.request(ShutdownReason::Sigterm, None)).await;
    let _ = finish_bootstrap.send(());

    wait_until("start skipped", || {
        records.has(Level::WARN, "Worker start skipped: shutting down")
    })
    .await;
    let warn = records
        .find("Worker start skipped: shutting down")
        .expect("warn logged");
    assert_eq!(warn.field("worker"), Some("chat"));
    assert_eq!(order.count("start"), 0);
    assert_eq!(exit.codes(), vec![0]);
}

/// Port of: apps/worker/__tests__/worker-shutdown.test.ts:296 "worker shutdown coordinator > a start that fails because shutdown began is not a startup failure"
#[tokio::test(start_paused = true)]
async fn a_start_that_fails_because_shutdown_began_is_not_a_startup() {
    let records = RecordingLog::install();
    let exit = ExitRecorder::new();
    let bootstraps = OrderLog::new();
    let shutdown = Shutdown::new(
        None,
        exit.exit_fn(),
        recording_bootstrap(bootstraps.clone()),
    );
    let (fail_start, started) = oneshot::channel::<()>();

    let _run = shutdown.run_worker(
        "schedule",
        Box::new(move || {
            Box::pin(async move {
                let _ = started.await;
                Err(err(
                    "Cannot start schedule worker: shutdown already requested",
                ))
            })
        }),
    );
    wait_until("bootstrap", || bootstraps.count("bootstrap") == 1).await;

    within(shutdown.request(ShutdownReason::Sigterm, None)).await;
    let _ = fail_start.send(());

    wait_until("start aborted", || {
        records.has(Level::WARN, "Worker start aborted by shutdown")
    })
    .await;
    let warn = records
        .find("Worker start aborted by shutdown")
        .expect("warn logged");
    assert_eq!(warn.field("worker"), Some("schedule"));
    assert!(records.at(Level::ERROR).is_empty());
    assert_eq!(exit.codes(), vec![0]);
}

/// Port of: apps/worker/__tests__/worker-shutdown.test.ts:316 "worker shutdown coordinator > installs signal and crash handlers once and routes crashes to exit 1"
#[tokio::test(start_paused = true)]
async fn installs_signal_and_crash_handlers_once_and_routes_crashes() {
    let records = RecordingLog::install();
    let exit = ExitRecorder::new();
    let shutdown = shutdown_with(&exit);
    let log = OrderLog::new();
    shutdown.on_shutdown("chat", ok_closer(&log, "chat"));
    shutdown.on_shutdown("low", ok_closer(&log, "low"));

    let mut handlers = shutdown.installed_handlers();
    handlers.sort_unstable();
    assert_eq!(
        handlers,
        vec![
            "SIGINT",
            "SIGTERM",
            "uncaughtException",
            "unhandledRejection"
        ]
    );

    let crash = shutdown.report_crash(CrashKind::UnhandledRejection, err("lost promise"));
    let _crash = tokio::spawn(crash);
    wait_until("exit(1)", || exit.codes().contains(&1)).await;
    let fatal = records.find("Unhandled rejection").expect("fatal logged");
    assert_eq!(fatal.level, Level::ERROR);
    assert!(fatal.field("err").is_some());
}

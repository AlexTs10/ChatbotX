//! Port of `apps/worker/__tests__/events-worker-shutdown.test.ts` (spec 5.3
//! "events-worker-shutdown"): the events worker entry under the shutdown
//! coordinator.

#[path = "support/shutdown.rs"]
mod shutdown_support;
mod support;

use std::sync::{Arc, Mutex};

use chatbotx_jobs::events::entry::{EventWorkerControl, RealtimeFlush, run_events_worker};
use chatbotx_jobs::shutdown::ShutdownReason;
use chatbotx_jobs::{BoxError, BoxFuture};
use shutdown_support::{ExitRecorder, OrderLog, err, shutdown_with, wait_until, within};
use support::RecordingLog;
use tokio::sync::oneshot;
use tracing::Level;

/// The mocked `@chatbotx.io/event-bus/worker` (`startWorker` / `stopWorker`).
struct FakeEventWorker {
    order: OrderLog,
    start_error: Option<&'static str>,
    start_gate: Mutex<Option<oneshot::Receiver<()>>>,
    stop_errors: Mutex<Vec<&'static str>>,
}

impl FakeEventWorker {
    fn new(order: &OrderLog) -> FakeEventWorker {
        FakeEventWorker {
            order: order.clone(),
            start_error: None,
            start_gate: Mutex::new(None),
            stop_errors: Mutex::new(Vec::new()),
        }
    }
}

impl EventWorkerControl for FakeEventWorker {
    fn start(&self) -> BoxFuture<'static, Result<(), BoxError>> {
        self.order.push("start");
        let gate = self.start_gate.lock().unwrap().take();
        let error = self.start_error;
        Box::pin(async move {
            if let Some(gate) = gate {
                let _ = gate.await;
            }
            match error {
                Some(message) => Err(err(message)),
                None => Ok(()),
            }
        })
    }

    fn stop(&self) -> BoxFuture<'static, Result<(), BoxError>> {
        self.order.push("stop");
        let error = self.stop_errors.lock().unwrap().pop();
        Box::pin(async move {
            match error {
                Some(message) => Err(err(message)),
                None => Ok(()),
            }
        })
    }
}

fn flush(order: &OrderLog) -> RealtimeFlush {
    let order = order.clone();
    Arc::new(move || {
        order.push("flush");
        Box::pin(async { Ok(()) }) as BoxFuture<'static, Result<(), BoxError>>
    })
}

/// Port of: apps/worker/__tests__/events-worker-shutdown.test.ts:44 "drains realtime events after the event worker stop fails"
#[tokio::test(start_paused = true)]
async fn drains_realtime_events_after_the_event_worker_stop_fails() {
    let exit = ExitRecorder::new();
    let shutdown = shutdown_with(&exit);
    let order = OrderLog::new();
    let worker = FakeEventWorker::new(&order);
    worker.stop_errors.lock().unwrap().push("stop failed");

    let started = run_events_worker(&shutdown, Arc::new(worker), flush(&order));
    within(started).await.expect("entry task");

    let _drain = tokio::spawn(shutdown.request(ShutdownReason::Sigterm, None));
    wait_until("exit(1)", || exit.codes().contains(&1)).await;

    assert_eq!(order.count("flush"), 1);
    let events = order.events();
    let stop = events
        .iter()
        .position(|e| e == "stop")
        .expect("stop called");
    let flushed = events
        .iter()
        .position(|e| e == "flush")
        .expect("flush called");
    assert!(stop < flushed, "{events:?}");
}

/// Port of: apps/worker/__tests__/events-worker-shutdown.test.ts:63 "a startWorker rejection takes the failed-to-start path"
#[tokio::test(start_paused = true)]
async fn a_startworker_rejection_takes_the_failed_to_start_path() {
    let records = RecordingLog::install();
    let exit = ExitRecorder::new();
    let shutdown = shutdown_with(&exit);
    let order = OrderLog::new();
    let mut worker = FakeEventWorker::new(&order);
    worker.start_error = Some("redis unavailable");

    let _entry = run_events_worker(&shutdown, Arc::new(worker), flush(&order));

    wait_until("Failed to start worker", || {
        records.has(Level::ERROR, "Failed to start worker")
    })
    .await;
    let error = records
        .find("Failed to start worker")
        .expect("error logged");
    assert_eq!(error.field("worker"), Some("events"));
    wait_until("exit(1)", || exit.codes().contains(&1)).await;
}

/// Port of: apps/worker/__tests__/events-worker-shutdown.test.ts:84 "stops consumers a shutdown missed because it began mid-startup"
#[tokio::test(start_paused = true)]
async fn stops_consumers_a_shutdown_missed_because_it_began_mid() {
    let exit = ExitRecorder::new();
    let shutdown = shutdown_with(&exit);
    let order = OrderLog::new();
    let worker = FakeEventWorker::new(&order);
    let (finish_start, gate) = oneshot::channel::<()>();
    *worker.start_gate.lock().unwrap() = Some(gate);

    let _entry = run_events_worker(&shutdown, Arc::new(worker), flush(&order));
    wait_until("start called", || order.count("start") == 1).await;

    // The closer stops the partial set; modules started after it still consume.
    let _drain = tokio::spawn(shutdown.request(ShutdownReason::Sigterm, None));
    wait_until("first stop", || order.count("stop") == 1).await;
    let _ = finish_start.send(());

    wait_until("second stop", || order.count("stop") == 2).await;
}

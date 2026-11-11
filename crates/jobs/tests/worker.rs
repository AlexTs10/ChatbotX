//! Port of `packages/event-bus/__tests__/worker.test.ts` (spec 5.3
//! "worker (event-bus)"): event worker lifecycle.

#[path = "support/events.rs"]
mod events_support;
mod support;

use std::sync::Arc;

use chatbotx_jobs::events::{EventModule, EventWorker, Listeners, default_consumer_name};
use events_support::{FakeConsumerBus, within};

fn module(bus: &Arc<FakeConsumerBus>) -> EventModule {
    EventModule {
        bus: Arc::clone(bus) as _,
        listeners: Listeners::new(),
    }
}

/// `TIMESTAMPED_WORKER_NAME = /^worker-worker-host-\d+-\d+$/`.
fn is_timestamped(name: &str) -> bool {
    let Some(rest) = name.strip_prefix("worker-worker-host-") else {
        return false;
    };
    let mut parts = rest.split('-');
    let (Some(a), Some(b), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|c| c.is_ascii_digit());
    digits(a) && digits(b)
}

/// Port of: packages/event-bus/__tests__/worker.test.ts:33 "event worker lifecycle > uses a stable default consumer name without a timestamp segment"
#[test]
fn uses_a_stable_default_consumer_name_without_a_timestamp() {
    let pid = std::process::id();
    let name = default_consumer_name(Some("worker-host"), pid);

    assert_eq!(name, format!("worker-worker-host-{pid}"));
    assert!(!is_timestamped(&name));
}

/// Port of: packages/event-bus/__tests__/worker.test.ts:58 "event worker lifecycle > stops consumers before deregistering them on shutdown"
#[tokio::test]
async fn stops_consumers_before_deregistering_them_on_shutdown() {
    let bus = FakeConsumerBus::new();
    let worker = EventWorker::new();
    within(worker.start(vec![module(&bus)], Some("consumer-1")))
        .await
        .expect("started");

    let stopped = worker.stop();

    assert_eq!(bus.count("stop"), 1);
    assert_eq!(bus.count("deregister"), 0);

    within(stopped).await;

    assert_eq!(bus.count("deregister"), 1);
}

/// Port of: packages/event-bus/__tests__/worker.test.ts:72 "event worker lifecycle > waits for consume loops to drain before deregistering"
#[tokio::test]
async fn waits_for_consume_loops_to_drain_before_deregistering() {
    let bus = FakeConsumerBus::looping_until_stop();
    let worker = EventWorker::new();
    within(worker.start(vec![module(&bus)], Some("consumer-1")))
        .await
        .expect("started");
    within(worker.stop()).await;

    assert_eq!(bus.count("stop"), 1);
    assert_eq!(bus.count("deregister"), 1);
    let log = bus.log();
    let stop = log.iter().position(|e| e == "stop").expect("stop");
    let deregister = log
        .iter()
        .position(|e| e == "deregister")
        .expect("deregister");
    assert!(stop < deregister);
}

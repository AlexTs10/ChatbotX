//! Port of `packages/event-bus/__tests__/event-bus.test.ts` (spec 5.3
//! "event-bus"): `BaseEventBus` consumer reliability. The fake Redis of
//! the original is a scripted `EventStore` fake (`support/events.rs`); each
//! Redis command maps to one store call (XREADGROUP → `read_new`,
//! XAUTOCLAIM → `reclaim`, XPENDING → `pending_count`, XACK → `ack`,
//! XADD → `append` / `dead_letter`, DELCONSUMER → `delete_consumer`,
//! duplicate connection `quit` / `disconnect` → `close_reader` /
//! `force_close_reader`).

#[path = "support/events.rs"]
mod events_support;
mod support;

use std::sync::{Arc, Mutex};

use chatbotx_jobs::events::{
    EventBus, EventBusConfig, EventListener, EventStoreError, ListenerFn, ListenerOutcome,
    Listeners, event_bus_message_id_key,
};
use chatbotx_jobs::shutdown::CancelToken;
use chatbotx_jobs::{BoxError, BoxFuture};
use events_support::{FakeEventStore, StoreCall, entry, test_config, within};
use serde_json::{Value, json};
use support::{RecordingLog, TestClock};
use tracing::Level;

fn bus(store: &Arc<FakeEventStore>, config: EventBusConfig) -> Arc<EventBus> {
    Arc::new(EventBus::new(
        Arc::clone(store) as _,
        config,
        TestClock::new(0),
    ))
}

fn bus_at(store: &Arc<FakeEventStore>, config: EventBusConfig, now_ms: i64) -> Arc<EventBus> {
    Arc::new(EventBus::new(
        Arc::clone(store) as _,
        config,
        TestClock::new(now_ms),
    ))
}

/// `claimIntervalMs: Number.POSITIVE_INFINITY`.
fn no_reclaim() -> EventBusConfig {
    EventBusConfig {
        claim_interval_ms: None,
        ..test_config()
    }
}

fn listeners(name: &str, handler: ListenerFn) -> Listeners {
    let mut map = Listeners::new();
    map.insert(
        "test:event".to_owned(),
        vec![EventListener {
            name: name.to_owned(),
            handler,
        }],
    );
    map
}

fn handler(
    f: impl Fn(Vec<Value>, CancelToken) -> BoxFuture<'static, Result<ListenerOutcome, BoxError>>
    + Send
    + Sync
    + 'static,
) -> ListenerFn {
    Arc::new(f)
}

/// A handler that never settles and counts its calls.
fn never_resolves(calls: Arc<Mutex<u32>>) -> ListenerFn {
    handler(move |_payloads, _signal| {
        *calls.lock().unwrap() += 1;
        Box::pin(std::future::pending())
    })
}

/// `vi.fn()`: resolves `undefined`, counts calls.
fn counting(calls: Arc<Mutex<u32>>) -> ListenerFn {
    handler(move |_payloads, _signal| {
        *calls.lock().unwrap() += 1;
        Box::pin(async { Ok(ListenerOutcome::default()) })
    })
}

/// Stops `bus` on the first read (`blockingRedis.call` → `bus.stop()`).
fn stop_on_first_read(store: &Arc<FakeEventStore>, bus: &Arc<EventBus>) {
    let token = bus.stop_token();
    store.on_read(move |n| {
        if n == 1 {
            token.cancel();
        }
    });
}

fn delete_consumer_called(store: &FakeEventStore) -> bool {
    store.calls().contains(&StoreCall::DeleteConsumer {
        stream: "events:test".to_owned(),
        group: "test-group".to_owned(),
        consumer: "consumer-1".to_owned(),
    })
}

fn pending_call() -> StoreCall {
    StoreCall::PendingCount {
        stream: "events:test".to_owned(),
        group: "test-group".to_owned(),
        consumer: "consumer-1".to_owned(),
    }
}

fn read_call(count: u32) -> StoreCall {
    StoreCall::ReadNew {
        stream: "events:test".to_owned(),
        group: "test-group".to_owned(),
        consumer: "consumer-1".to_owned(),
        count,
    }
}

/// Port of: packages/event-bus/__tests__/event-bus.test.ts:82 "BaseEventBus consumer reliability > uses a dedicated duplicate connection for blocking reads and closes it on deregister"
#[tokio::test]
async fn uses_a_dedicated_duplicate_connection_for_blocking_reads() {
    let store = FakeEventStore::new();
    let bus = bus(&store, no_reclaim());
    store.pending_default(Ok(0));
    stop_on_first_read(&store, &bus);

    within(bus.start_consuming("consumer-1", Listeners::new())).await;
    within(bus.deregister()).await;

    // The consume loop waits on its wake-up signal (no BLOCK poll); the
    // dedicated reader is released exactly once.
    assert!(store.reads().contains(&read_call(500)));
    assert!(store.calls().contains(&pending_call()));
    assert!(delete_consumer_called(&store));
    assert_eq!(
        store.count(
            |c| matches!(c, StoreCall::CloseReader { consumer } if consumer == "consumer-1")
        ),
        1
    );
}

/// Port of: packages/event-bus/__tests__/event-bus.test.ts:131 "BaseEventBus consumer reliability > keeps the consumer registered when pending messages remain"
#[tokio::test]
async fn keeps_the_consumer_registered_when_pending_messages_remain() {
    let log = RecordingLog::install();
    let store = FakeEventStore::new();
    store.pending_default(Ok(1));
    let bus = bus(&store, no_reclaim());
    stop_on_first_read(&store, &bus);

    within(bus.start_consuming("consumer-1", Listeners::new())).await;
    within(bus.deregister()).await;

    assert!(store.calls().contains(&pending_call()));
    assert!(!delete_consumer_called(&store));
    let warn = log
        .find("[EventBus] skipping DELCONSUMER because messages are still pending")
        .expect("warn logged");
    assert_eq!(warn.level, Level::WARN);
    assert_eq!(warn.field("pending"), Some("1"));
    assert_eq!(warn.field("stream"), Some("events:test"));
    assert_eq!(warn.field("consumer_name"), Some("consumer-1"));
}

/// Port of: packages/event-bus/__tests__/event-bus.test.ts:174 "BaseEventBus consumer reliability > keeps the consumer registered when pending inspection fails"
#[tokio::test]
async fn keeps_the_consumer_registered_when_pending_inspection_fails() {
    let log = RecordingLog::install();
    let store = FakeEventStore::new();
    store.pending_default(Err(EventStoreError::Other(
        "pending check failed".to_owned(),
    )));
    let bus = bus(&store, no_reclaim());
    stop_on_first_read(&store, &bus);

    within(bus.start_consuming("consumer-1", Listeners::new())).await;
    within(bus.deregister()).await;

    assert!(!delete_consumer_called(&store));
    let error = log
        .find("[EventBus] skipping DELCONSUMER after XPENDING failed")
        .expect("error logged");
    assert_eq!(error.level, Level::ERROR);
    assert!(error.field("err").is_some());
    assert_eq!(error.field("stream"), Some("events:test"));
    assert_eq!(error.field("consumer_name"), Some("consumer-1"));
}

/// Port of: packages/event-bus/__tests__/event-bus.test.ts:210 "BaseEventBus consumer reliability > initializes a consumer group once and ignores existing groups"
#[tokio::test]
async fn initializes_a_consumer_group_once_and_ignores_existing_groups() {
    let store = FakeEventStore::new();
    let bus = bus(&store, test_config());

    assert_eq!(bus.initialize().await, Ok(()));
    assert_eq!(bus.initialize().await, Ok(()));

    assert_eq!(
        store.create_groups(),
        vec![StoreCall::CreateGroup {
            stream: "events:test".to_owned(),
            group: "test-group".to_owned(),
        }]
    );

    let existing = FakeEventStore::new();
    existing.create_group_default(Err(EventStoreError::BusyGroup));
    let existing_bus = self::bus(&existing, test_config());
    assert_eq!(existing_bus.initialize().await, Ok(()));
}

/// Port of: packages/event-bus/__tests__/event-bus.test.ts:232 "BaseEventBus consumer reliability > throws initialization errors that are not BUSYGROUP"
#[tokio::test]
async fn throws_initialization_errors_that_are_not_busygroup() {
    let store = FakeEventStore::new();
    store.create_group_default(Err(EventStoreError::Other("redis down".to_owned())));

    let err = bus(&store, test_config())
        .initialize()
        .await
        .expect_err("initialize rejects");
    assert!(err.to_string().contains("redis down"), "{err:?}");
}

/// Port of: packages/event-bus/__tests__/event-bus.test.ts:239 "BaseEventBus consumer reliability > emits transformed payloads with maxlen and rejects invalid payloads"
#[tokio::test]
async fn emits_transformed_payloads_with_maxlen_and_rejects_invalid() {
    let store = FakeEventStore::new();
    let upper = EventBus::new(
        Arc::clone(&store) as _,
        EventBusConfig {
            max_len: 42,
            ..test_config()
        },
        TestClock::new(1234),
    )
    .with_payload_handler(Arc::new(|_event_type, payload| {
        let value = payload["value"].as_str().unwrap_or_default().to_uppercase();
        json!({ "value": value })
    }));

    assert_eq!(
        upper.emit("test:event", json!({ "value": "ok" })).await,
        Ok("1-0".to_owned())
    );
    let validating = bus_at(&store, test_config(), 1234);
    assert_eq!(
        validating.emit("test:event", json!({ "value": 123 })).await,
        Ok(String::new())
    );

    assert_eq!(
        store.appends(),
        vec![StoreCall::Append {
            stream: "events:test".to_owned(),
            max_len: 42,
            event_type: "test:event".to_owned(),
            payload: json!({ "value": "OK" }).to_string(),
            ts_ms: 1234,
        }]
    );
}

/// Port of: packages/event-bus/__tests__/event-bus.test.ts:270 "BaseEventBus consumer reliability > clones a bus with a different consumer group"
#[test]
fn clones_a_bus_with_a_different_consumer_group() {
    let store = FakeEventStore::new();
    let clone = bus(
        &store,
        EventBusConfig {
            read_batch_size: 25,
            ..test_config()
        },
    )
    .clone_for_group("other-group");

    assert_eq!(clone.config().consumer_group, "other-group");
    assert_eq!(clone.config().read_batch_size, 25);
    assert_eq!(clone.config().stream_key, "events:test");
}

/// Port of: packages/event-bus/__tests__/event-bus.test.ts:283 "BaseEventBus consumer reliability > uses configured read batch size for XREADGROUP"
#[tokio::test]
async fn uses_configured_read_batch_size_for_xreadgroup() {
    let store = FakeEventStore::new();
    let bus = bus(
        &store,
        EventBusConfig {
            read_batch_size: 123,
            ..no_reclaim()
        },
    );

    within(bus.consume_once("consumer-1", &Listeners::new()))
        .await
        .expect("consumed");

    assert_eq!(store.reads(), vec![read_call(123)]);
}

/// Port of: packages/event-bus/__tests__/event-bus.test.ts:307 "BaseEventBus consumer reliability > leaves a timed-out batch unacked and allows the next read attempt"
#[tokio::test(start_paused = true)]
async fn leaves_a_timed_out_batch_unacked_and_allows_the_next_read() {
    let log = RecordingLog::install();
    let store = FakeEventStore::new();
    store.read_once(Ok(vec![entry("1-0", "stuck", 1)]));
    store.read_once(Ok(Vec::new()));
    let bus = bus(
        &store,
        EventBusConfig {
            process_timeout_ms: 25,
            ..no_reclaim()
        },
    );
    let calls = Arc::new(Mutex::new(0));
    let stuck = listeners("stuck-listener", never_resolves(Arc::clone(&calls)));

    within(bus.consume_once("consumer-1", &stuck))
        .await
        .expect("first read");
    within(bus.consume_once("consumer-1", &Listeners::new()))
        .await
        .expect("second read");

    assert!(store.acks().is_empty());
    assert_eq!(store.reads().len(), 2);
    let warn = log
        .find("[EventBus] batch processing timed out; leaving unacked for reclaim")
        .expect("warn logged");
    assert_eq!(warn.level, Level::WARN);
    assert_eq!(warn.field("count"), Some("1"));
    assert_eq!(warn.field("stream"), Some("events:test"));
    assert_eq!(warn.field("timeout_ms"), Some("25"));
}

/// Port of: packages/event-bus/__tests__/event-bus.test.ts:343 "BaseEventBus consumer reliability > passes an AbortSignal to handlers and aborts it on timeout"
#[tokio::test(start_paused = true)]
async fn passes_an_abortsignal_to_handlers_and_aborts_it_on_timeout() {
    let store = FakeEventStore::new();
    store.read_once(Ok(vec![entry("1-0", "stuck", 1)]));
    let bus = bus(
        &store,
        EventBusConfig {
            process_timeout_ms: 25,
            ..no_reclaim()
        },
    );
    let received: Arc<Mutex<Option<CancelToken>>> = Arc::new(Mutex::new(None));
    let seen = Arc::clone(&received);
    let aware = listeners(
        "abort-aware-listener",
        handler(move |_payloads, signal| {
            *seen.lock().unwrap() = Some(signal);
            Box::pin(std::future::pending())
        }),
    );

    within(bus.consume_once("consumer-1", &aware))
        .await
        .expect("read");

    let signal = received.lock().unwrap().clone().expect("signal passed");
    assert!(signal.is_cancelled());
    assert!(store.acks().is_empty());
}

/// Port of: packages/event-bus/__tests__/event-bus.test.ts:370 "BaseEventBus consumer reliability > rate-limits XAUTOCLAIM and processes claimed entries through ack path"
#[tokio::test]
async fn rate_limits_xautoclaim_and_processes_claimed_entries() {
    let store = FakeEventStore::new();
    store.reclaim_default(Ok(vec![entry("2-0", "claimed", 1)]));
    let bus = bus_at(
        &store,
        EventBusConfig {
            claim_batch_size: 77,
            claim_idle_ms: 180_000,
            claim_interval_ms: Some(30_000),
            ..test_config()
        },
        1000,
    );

    within(bus.consume_once("consumer-1", &Listeners::new()))
        .await
        .expect("first");
    within(bus.consume_once("consumer-1", &Listeners::new()))
        .await
        .expect("second");

    assert_eq!(
        store.reclaims(),
        vec![StoreCall::Reclaim {
            stream: "events:test".to_owned(),
            group: "test-group".to_owned(),
            consumer: "consumer-1".to_owned(),
            min_idle_ms: 180_000,
            count: 77,
        }]
    );
    assert!(store.acked_ids().contains(&"2-0".to_owned()));
}

/// Port of: packages/event-bus/__tests__/event-bus.test.ts:401 "BaseEventBus consumer reliability > logs handler failures before acking a completed batch"
#[tokio::test]
async fn logs_handler_failures_before_acking_a_completed_batch() {
    let log = RecordingLog::install();
    let store = FakeEventStore::new();
    store.read_once(Ok(vec![entry("3-0", "failed", 1)]));
    let bus = bus(&store, no_reclaim());
    let failing = listeners(
        "failing-listener",
        handler(|_payloads, _signal| {
            Box::pin(async { Err::<ListenerOutcome, BoxError>("listener failed".into()) })
        }),
    );

    within(bus.consume_once("consumer-1", &failing))
        .await
        .expect("consumed");

    let warn = log
        .find("[EventBus] acking batch despite handler failures")
        .expect("warn logged");
    assert_eq!(warn.level, Level::WARN);
    assert_eq!(warn.field("failed"), Some("1"));
    assert_eq!(warn.field("stream"), Some("events:test"));
    assert_eq!(warn.field("total"), Some("1"));
    assert!(store.acked_ids().contains(&"3-0".to_owned()));
}

/// Port of: packages/event-bus/__tests__/event-bus.test.ts:428 "BaseEventBus consumer reliability > attaches stream ids and selectively acks only successful messages when enabled"
#[tokio::test]
async fn attaches_stream_ids_and_selectively_acks_only_successful() {
    let store = FakeEventStore::new();
    store.read_once(Ok(vec![entry("4-0", "ok", 1), entry("5-0", "retry", 1)]));
    let bus = bus(
        &store,
        EventBusConfig {
            enable_selective_retry: Some(true),
            ..no_reclaim()
        },
    );
    let received: Arc<Mutex<Vec<Vec<Value>>>> = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&received);
    let partial = listeners(
        "partial-listener",
        handler(move |payloads, _signal| {
            seen.lock().unwrap().push(payloads);
            Box::pin(async {
                Ok(ListenerOutcome {
                    failed_message_ids: vec!["5-0".to_owned()],
                })
            })
        }),
    );

    within(bus.consume_once("consumer-1", &partial))
        .await
        .expect("consumed");

    let key = event_bus_message_id_key();
    let mut ok = json!({ "value": "ok" });
    ok[key] = json!("4-0");
    let mut retry = json!({ "value": "retry" });
    retry[key] = json!("5-0");
    assert_eq!(received.lock().unwrap().clone(), vec![vec![ok, retry]]);
    assert_eq!(store.acks(), vec![vec!["4-0".to_owned()]]);
}

/// Port of: packages/event-bus/__tests__/event-bus.test.ts:459 "BaseEventBus consumer reliability > moves reclaimed messages past max deliveries to the dead-letter stream"
#[tokio::test]
async fn moves_reclaimed_messages_past_max_deliveries_to_the_dead() {
    // 2026-07-01T00:00:00.000Z
    let now = 1_782_864_000_000;
    let store = FakeEventStore::new();
    store.reclaim_default(Ok(vec![entry("6-0", "poison", 3)]));
    let bus = bus_at(
        &store,
        EventBusConfig {
            claim_interval_ms: Some(0),
            dead_letter_max_len: 10,
            enable_selective_retry: Some(true),
            max_deliveries: 2,
            ..test_config()
        },
        now,
    );
    let calls = Arc::new(Mutex::new(0));
    let not_called = listeners("not-called", counting(Arc::clone(&calls)));

    within(bus.consume_once("consumer-1", &not_called))
        .await
        .expect("consumed");

    assert_eq!(*calls.lock().unwrap(), 0);
    let dead = store.dead_letters();
    assert_eq!(dead.len(), 1);
    let dead = &dead[0];
    assert_eq!(dead.stream, "events:test:dead");
    assert_eq!(dead.max_len, 10);
    assert_eq!(dead.event_type, "test:event");
    assert_eq!(dead.payload, json!({ "value": "poison" }).to_string());
    assert_eq!(dead.original_id, "6-0");
    assert_eq!(dead.delivery_count, 3);
    assert_eq!(dead.ts_ms, now);
    assert!(store.acked_ids().contains(&"6-0".to_owned()));
}

/// Port of: packages/event-bus/__tests__/event-bus.test.ts:509 "BaseEventBus consumer reliability > reprocesses reclaimed messages at or below max deliveries"
#[tokio::test]
async fn reprocesses_reclaimed_messages_at_or_below_max_deliveries() {
    let store = FakeEventStore::new();
    store.reclaim_default(Ok(vec![entry("7-0", "retryable", 2)]));
    let bus = bus(
        &store,
        EventBusConfig {
            claim_interval_ms: Some(0),
            enable_selective_retry: Some(true),
            max_deliveries: 2,
            ..test_config()
        },
    );
    let calls = Arc::new(Mutex::new(0));
    let retryable = listeners("retryable-listener", counting(Arc::clone(&calls)));

    within(bus.consume_once("consumer-1", &retryable))
        .await
        .expect("consumed");

    assert_eq!(*calls.lock().unwrap(), 1);
    assert!(store.dead_letters().is_empty());
    assert!(store.appends().is_empty());
    assert!(store.acked_ids().contains(&"7-0".to_owned()));
}

/// Port of: packages/event-bus/__tests__/event-bus.test.ts:538 "BaseEventBus consumer reliability > writes the stored processing error to the dead-letter stream"
#[tokio::test]
async fn writes_the_stored_processing_error_to_the_dead_letter_stream() {
    let store = FakeEventStore::new();
    store.reclaim_once(Ok(Vec::new()));
    store.reclaim_default(Ok(vec![entry("8-0", "poison", 3)]));
    store.read_once(Ok(vec![entry("8-0", "poison", 1)]));
    let bus = bus(
        &store,
        EventBusConfig {
            claim_interval_ms: Some(0),
            enable_selective_retry: Some(true),
            max_deliveries: 2,
            ..test_config()
        },
    );
    let poison = listeners(
        "poison-listener",
        handler(|_payloads, _signal| {
            Box::pin(async { Err::<ListenerOutcome, BoxError>("database unavailable".into()) })
        }),
    );

    within(bus.consume_once("consumer-1", &poison))
        .await
        .expect("first");
    within(bus.consume_once("consumer-1", &Listeners::new()))
        .await
        .expect("second");

    let dead = store.dead_letters();
    assert!(
        dead.iter().any(|d| d.event_type == "test:event"
            && d.original_id == "8-0"
            && d.delivery_count == 3
            && d.error == "database unavailable"),
        "{dead:?}"
    );
}

/// Port of: packages/event-bus/__tests__/event-bus.test.ts:592 "BaseEventBus consumer reliability > writes a max-deliveries fallback error after restart-like empty error memory"
#[tokio::test]
async fn writes_a_max_deliveries_fallback_error_after_restart_like() {
    let log = RecordingLog::install();
    let store = FakeEventStore::new();
    store.reclaim_default(Ok(vec![entry("9-0", "poison", 6)]));
    let bus = bus(
        &store,
        EventBusConfig {
            claim_interval_ms: Some(0),
            enable_selective_retry: Some(true),
            max_deliveries: 5,
            ..test_config()
        },
    );

    within(bus.consume_once("consumer-1", &Listeners::new()))
        .await
        .expect("consumed");

    let dead = store.dead_letters();
    assert!(
        dead.iter().any(|d| d.event_type == "test:event"
            && d.original_id == "9-0"
            && d.delivery_count == 6
            && d.error == "max deliveries exceeded"),
        "{dead:?}"
    );
    let error = log
        .find("[EventBus] moved message to dead-letter stream")
        .expect("dead-letter log");
    assert_eq!(error.level, Level::ERROR);
    assert_eq!(
        error.field("dead_letter_stream_key"),
        Some("events:test:dead")
    );
    assert_eq!(error.field("delivery_count"), Some("6"));
    assert_eq!(error.field("message_id"), Some("9-0"));
    assert_eq!(error.field("stream"), Some("events:test"));
}

/// Port of: packages/event-bus/__tests__/event-bus.test.ts:647 "BaseEventBus consumer reliability > logs reclaim failures and continues with the normal read"
#[tokio::test]
async fn logs_reclaim_failures_and_continues_with_the_normal_read() {
    let log = RecordingLog::install();
    let store = FakeEventStore::new();
    store.reclaim_default(Err(EventStoreError::Other("claim failed".to_owned())));
    let bus = bus(&store, test_config());

    let _ = within(bus.consume_once("consumer-1", &Listeners::new())).await;

    let error = log.find("[EventBus] reclaim failed").expect("error logged");
    assert_eq!(error.level, Level::ERROR);
    assert!(error.field("err").is_some());
    assert_eq!(error.field("stream"), Some("events:test"));
    assert!(store.reads().contains(&read_call(500)));
}

/// Port of: packages/event-bus/__tests__/event-bus.test.ts:677 "BaseEventBus consumer reliability > falls back to disconnect when closing a duplicate connection fails"
#[tokio::test]
async fn falls_back_to_disconnect_when_closing_a_duplicate_connection() {
    let store = FakeEventStore::new();
    store.close_reader_default(Err(EventStoreError::Other("quit failed".to_owned())));
    let bus = bus(&store, no_reclaim());
    stop_on_first_read(&store, &bus);

    within(bus.start_consuming("consumer-1", Listeners::new())).await;

    assert_eq!(
        store.count(|c| matches!(c, StoreCall::CloseReader { .. })),
        1
    );
    assert_eq!(
        store.count(|c| matches!(c, StoreCall::ForceCloseReader { .. })),
        1
    );
}

/// Port of: packages/event-bus/__tests__/event-bus.test.ts:697 "BaseEventBus consumer reliability > logs consume loop errors and retries while running"
#[tokio::test(start_paused = true)]
async fn logs_consume_loop_errors_and_retries_while_running() {
    let log = RecordingLog::install();
    let store = FakeEventStore::new();
    // First iteration fails (the overridden consumeOnce rejects), the retry
    // after the 1000 ms pause stops the bus.
    store.read_once(Err(EventStoreError::Other("read failed".to_owned())));
    let bus = bus(&store, no_reclaim());
    let token = bus.stop_token();
    store.on_read(move |n| {
        if n == 2 {
            token.cancel();
        }
    });

    let started = tokio::time::Instant::now();
    within(bus.start_consuming("consumer-1", Listeners::new())).await;

    assert!(started.elapsed() >= std::time::Duration::from_millis(1000));
    assert_eq!(store.reads().len(), 2);
    let error = log
        .find("[EventBus] consume loop error")
        .expect("error logged");
    assert_eq!(error.level, Level::ERROR);
    assert!(error.field("err").is_some());
    assert_eq!(error.field("stream"), Some("events:test"));
}

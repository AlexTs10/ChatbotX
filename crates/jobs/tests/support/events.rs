//! Event-bus test support: a scripted `EventStore` fake (replaces the fake
//! Redis of `event-bus.test.ts`), fake emitters (`index.test.ts`) and a fake
//! `ConsumerBus` (`worker.test.ts`). Complete test code.

#![allow(dead_code)]

use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chatbotx_jobs::BoxFuture;
use chatbotx_jobs::events::{
    ConsumerBus, DeadLetter, EventBusConfig, EventBusError, EventEmitter, EventStore,
    EventStoreError, Listeners, PayloadSchema, StreamEntry,
};
use serde_json::Value;
use tokio::sync::watch;

/// Awaits `fut`, failing the test after 5 s (virtual time when paused).
pub async fn within<T>(fut: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(5), fut)
        .await
        .expect("did not finish within 5 s")
}

/// `z.object({ value: z.string() })`.
pub fn test_event_schema(payload: &Value) -> bool {
    payload.get("value").is_some_and(Value::is_string)
}

/// The `createBus` config: stream `events:test`, group `test-group`, one
/// schema for `test:event`, every other field from the defaults.
pub fn test_config() -> EventBusConfig {
    let mut schemas: BTreeMap<String, PayloadSchema> = BTreeMap::new();
    schemas.insert("test:event".to_owned(), test_event_schema);
    EventBusConfig::new("events:test", "test-group", schemas)
}

/// `streamEntry(id, { value })` with a delivery count.
pub fn entry(id: &str, value: &str, delivery_count: u32) -> StreamEntry {
    StreamEntry {
        id: id.to_owned(),
        event_type: "test:event".to_owned(),
        payload: serde_json::json!({ "value": value }).to_string(),
        delivery_count,
    }
}

/// One recorded store call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreCall {
    CreateGroup {
        stream: String,
        group: String,
    },
    Append {
        stream: String,
        max_len: u64,
        event_type: String,
        payload: String,
        ts_ms: i64,
    },
    ReadNew {
        stream: String,
        group: String,
        consumer: String,
        count: u32,
    },
    Reclaim {
        stream: String,
        group: String,
        consumer: String,
        min_idle_ms: u64,
        count: u32,
    },
    PendingCount {
        stream: String,
        group: String,
        consumer: String,
    },
    Ack {
        stream: String,
        group: String,
        ids: Vec<String>,
    },
    DeadLetter(DeadLetter),
    DeleteConsumer {
        stream: String,
        group: String,
        consumer: String,
    },
    CloseReader {
        consumer: String,
    },
    ForceCloseReader {
        consumer: String,
    },
}

/// `mockResolvedValueOnce` values first, then the sticky default
/// (`mockResolvedValue`).
pub struct Script<T: Clone> {
    once: VecDeque<T>,
    default: T,
}

impl<T: Clone> Script<T> {
    pub fn new(default: T) -> Script<T> {
        Script {
            once: VecDeque::new(),
            default,
        }
    }
    fn next(&mut self) -> T {
        self.once
            .pop_front()
            .unwrap_or_else(|| self.default.clone())
    }
}

type Res<T> = Result<T, EventStoreError>;
type ReadHook = Box<dyn Fn(usize) + Send + Sync>;

/// The scripted fake store. Defaults match `createFakeRedis()`: group
/// create OK, append `"1-0"`, read and reclaim empty, 0 pending, close OK.
pub struct FakeEventStore {
    calls: Mutex<Vec<StoreCall>>,
    pub create_group: Mutex<Script<Res<()>>>,
    pub append: Mutex<Script<Res<String>>>,
    pub read: Mutex<Script<Res<Vec<StreamEntry>>>>,
    pub reclaim: Mutex<Script<Res<Vec<StreamEntry>>>>,
    pub pending: Mutex<Script<Res<u64>>>,
    pub close_reader: Mutex<Script<Res<()>>>,
    reads: AtomicUsize,
    on_read: Mutex<Option<ReadHook>>,
}

impl FakeEventStore {
    pub fn new() -> Arc<FakeEventStore> {
        Arc::new(FakeEventStore {
            calls: Mutex::new(Vec::new()),
            create_group: Mutex::new(Script::new(Ok(()))),
            append: Mutex::new(Script::new(Ok("1-0".to_owned()))),
            read: Mutex::new(Script::new(Ok(Vec::new()))),
            reclaim: Mutex::new(Script::new(Ok(Vec::new()))),
            pending: Mutex::new(Script::new(Ok(0))),
            close_reader: Mutex::new(Script::new(Ok(()))),
            reads: AtomicUsize::new(0),
            on_read: Mutex::new(None),
        })
    }

    pub fn read_once(&self, value: Res<Vec<StreamEntry>>) {
        self.read.lock().unwrap().once.push_back(value);
    }
    pub fn read_default(&self, value: Res<Vec<StreamEntry>>) {
        self.read.lock().unwrap().default = value;
    }
    pub fn reclaim_once(&self, value: Res<Vec<StreamEntry>>) {
        self.reclaim.lock().unwrap().once.push_back(value);
    }
    pub fn reclaim_default(&self, value: Res<Vec<StreamEntry>>) {
        self.reclaim.lock().unwrap().default = value;
    }
    pub fn create_group_default(&self, value: Res<()>) {
        self.create_group.lock().unwrap().default = value;
    }
    pub fn pending_default(&self, value: Res<u64>) {
        self.pending.lock().unwrap().default = value;
    }
    pub fn close_reader_default(&self, value: Res<()>) {
        self.close_reader.lock().unwrap().default = value;
    }

    /// Runs `hook(n)` on the n-th `read_new` (1-based), before it returns
    /// (the `blockingRedis.call.mockImplementation(() => bus.stop())` seam).
    pub fn on_read(&self, hook: impl Fn(usize) + Send + Sync + 'static) {
        *self.on_read.lock().unwrap() = Some(Box::new(hook));
    }

    pub fn calls(&self) -> Vec<StoreCall> {
        self.calls.lock().unwrap().clone()
    }

    pub fn reads(&self) -> Vec<StoreCall> {
        self.filter(|c| matches!(c, StoreCall::ReadNew { .. }))
    }
    pub fn reclaims(&self) -> Vec<StoreCall> {
        self.filter(|c| matches!(c, StoreCall::Reclaim { .. }))
    }
    pub fn appends(&self) -> Vec<StoreCall> {
        self.filter(|c| matches!(c, StoreCall::Append { .. }))
    }
    pub fn create_groups(&self) -> Vec<StoreCall> {
        self.filter(|c| matches!(c, StoreCall::CreateGroup { .. }))
    }
    /// The id lists of every `ack` call, in order.
    pub fn acks(&self) -> Vec<Vec<String>> {
        self.calls()
            .into_iter()
            .filter_map(|c| match c {
                StoreCall::Ack { ids, .. } => Some(ids),
                _ => None,
            })
            .collect()
    }
    pub fn acked_ids(&self) -> Vec<String> {
        self.acks().into_iter().flatten().collect()
    }
    pub fn dead_letters(&self) -> Vec<DeadLetter> {
        self.calls()
            .into_iter()
            .filter_map(|c| match c {
                StoreCall::DeadLetter(d) => Some(d),
                _ => None,
            })
            .collect()
    }
    pub fn count(&self, pred: impl Fn(&StoreCall) -> bool) -> usize {
        self.calls().iter().filter(|c| pred(c)).count()
    }

    fn filter(&self, pred: impl Fn(&StoreCall) -> bool) -> Vec<StoreCall> {
        self.calls().into_iter().filter(|c| pred(c)).collect()
    }

    fn record(&self, call: StoreCall) {
        self.calls.lock().unwrap().push(call);
    }
}

impl EventStore for FakeEventStore {
    fn create_group<'a>(&'a self, stream: &'a str, group: &'a str) -> BoxFuture<'a, Res<()>> {
        self.record(StoreCall::CreateGroup {
            stream: stream.to_owned(),
            group: group.to_owned(),
        });
        let r = self.create_group.lock().unwrap().next();
        Box::pin(async move { r })
    }

    fn append<'a>(
        &'a self,
        stream: &'a str,
        max_len: u64,
        event_type: &'a str,
        payload: &'a str,
        ts_ms: i64,
    ) -> BoxFuture<'a, Res<String>> {
        self.record(StoreCall::Append {
            stream: stream.to_owned(),
            max_len,
            event_type: event_type.to_owned(),
            payload: payload.to_owned(),
            ts_ms,
        });
        let r = self.append.lock().unwrap().next();
        Box::pin(async move { r })
    }

    fn read_new<'a>(
        &'a self,
        stream: &'a str,
        group: &'a str,
        consumer: &'a str,
        count: u32,
    ) -> BoxFuture<'a, Res<Vec<StreamEntry>>> {
        self.record(StoreCall::ReadNew {
            stream: stream.to_owned(),
            group: group.to_owned(),
            consumer: consumer.to_owned(),
            count,
        });
        let n = self.reads.fetch_add(1, Ordering::SeqCst) + 1;
        if let Some(hook) = self.on_read.lock().unwrap().as_ref() {
            hook(n);
        }
        let r = self.read.lock().unwrap().next();
        Box::pin(async move { r })
    }

    fn reclaim<'a>(
        &'a self,
        stream: &'a str,
        group: &'a str,
        consumer: &'a str,
        min_idle_ms: u64,
        count: u32,
    ) -> BoxFuture<'a, Res<Vec<StreamEntry>>> {
        self.record(StoreCall::Reclaim {
            stream: stream.to_owned(),
            group: group.to_owned(),
            consumer: consumer.to_owned(),
            min_idle_ms,
            count,
        });
        let r = self.reclaim.lock().unwrap().next();
        Box::pin(async move { r })
    }

    fn pending_count<'a>(
        &'a self,
        stream: &'a str,
        group: &'a str,
        consumer: &'a str,
    ) -> BoxFuture<'a, Res<u64>> {
        self.record(StoreCall::PendingCount {
            stream: stream.to_owned(),
            group: group.to_owned(),
            consumer: consumer.to_owned(),
        });
        let r = self.pending.lock().unwrap().next();
        Box::pin(async move { r })
    }

    fn ack<'a>(
        &'a self,
        stream: &'a str,
        group: &'a str,
        ids: Vec<String>,
    ) -> BoxFuture<'a, Res<u64>> {
        let n = ids.len() as u64;
        self.record(StoreCall::Ack {
            stream: stream.to_owned(),
            group: group.to_owned(),
            ids,
        });
        Box::pin(async move { Ok(n) })
    }

    fn dead_letter(&self, entry: DeadLetter) -> BoxFuture<'_, Res<String>> {
        self.record(StoreCall::DeadLetter(entry));
        Box::pin(async move { Ok("1-0".to_owned()) })
    }

    fn delete_consumer<'a>(
        &'a self,
        stream: &'a str,
        group: &'a str,
        consumer: &'a str,
    ) -> BoxFuture<'a, Res<()>> {
        self.record(StoreCall::DeleteConsumer {
            stream: stream.to_owned(),
            group: group.to_owned(),
            consumer: consumer.to_owned(),
        });
        Box::pin(async move { Ok(()) })
    }

    fn close_reader<'a>(&'a self, consumer: &'a str) -> BoxFuture<'a, Res<()>> {
        self.record(StoreCall::CloseReader {
            consumer: consumer.to_owned(),
        });
        let r = self.close_reader.lock().unwrap().next();
        Box::pin(async move { r })
    }

    fn force_close_reader(&self, consumer: &str) {
        self.record(StoreCall::ForceCloseReader {
            consumer: consumer.to_owned(),
        });
    }
}

// ---------------------------------------------------------------------------
// FakeEmitter (index.test.ts spies on each bus's emit)
// ---------------------------------------------------------------------------

/// Resolves to `id`, or fails synchronously when `sync_error` is set.
pub struct FakeEmitter {
    pub id: String,
    pub sync_error: Option<String>,
    pub calls: AtomicUsize,
}

impl FakeEmitter {
    pub fn resolving(id: &str) -> Arc<FakeEmitter> {
        Arc::new(FakeEmitter {
            id: id.to_owned(),
            sync_error: None,
            calls: AtomicUsize::new(0),
        })
    }

    pub fn throwing(message: &str) -> Arc<FakeEmitter> {
        Arc::new(FakeEmitter {
            id: String::new(),
            sync_error: Some(message.to_owned()),
            calls: AtomicUsize::new(0),
        })
    }

    pub fn call_count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl EventEmitter for FakeEmitter {
    fn emit<'a>(
        &'a self,
        _event_type: &'a str,
        _payload: Value,
    ) -> Result<BoxFuture<'a, Result<String, EventBusError>>, EventBusError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if let Some(message) = &self.sync_error {
            return Err(EventBusError::Other(message.clone()));
        }
        let id = self.id.clone();
        Ok(Box::pin(async move { Ok(id) }))
    }
}

// ---------------------------------------------------------------------------
// FakeConsumerBus (worker.test.ts createModule)
// ---------------------------------------------------------------------------

/// Records the order of `initialize`, `start_consuming:<consumer>:<n listeners>`,
/// `stop`, `deregister`. `start_consuming` resolves at once, or (with
/// `loop_until_stop`) only after `stop` was called.
pub struct FakeConsumerBus {
    pub log: Mutex<Vec<String>>,
    loop_until_stop: bool,
    stopped: watch::Sender<bool>,
}

impl FakeConsumerBus {
    pub fn new() -> Arc<FakeConsumerBus> {
        FakeConsumerBus::build(false)
    }

    pub fn looping_until_stop() -> Arc<FakeConsumerBus> {
        FakeConsumerBus::build(true)
    }

    fn build(loop_until_stop: bool) -> Arc<FakeConsumerBus> {
        let (stopped, _) = watch::channel(false);
        Arc::new(FakeConsumerBus {
            log: Mutex::new(Vec::new()),
            loop_until_stop,
            stopped,
        })
    }

    pub fn log(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }

    pub fn count(&self, prefix: &str) -> usize {
        self.log().iter().filter(|e| e.starts_with(prefix)).count()
    }
}

impl ConsumerBus for FakeConsumerBus {
    fn initialize(&self) -> BoxFuture<'_, Result<(), EventBusError>> {
        self.log.lock().unwrap().push("initialize".to_owned());
        Box::pin(async { Ok(()) })
    }

    fn start_consuming(
        self: Arc<Self>,
        consumer: String,
        listeners: Listeners,
    ) -> BoxFuture<'static, ()> {
        self.log
            .lock()
            .unwrap()
            .push(format!("start_consuming:{consumer}:{}", listeners.len()));
        let mut rx = self.stopped.subscribe();
        let wait = self.loop_until_stop;
        Box::pin(async move {
            if wait {
                let _ = rx.wait_for(|stopped| *stopped).await;
            }
        })
    }

    fn stop(&self) {
        self.log.lock().unwrap().push("stop".to_owned());
        self.stopped.send_replace(true);
    }

    fn deregister(&self) -> BoxFuture<'_, Result<(), EventBusError>> {
        self.log.lock().unwrap().push("deregister".to_owned());
        Box::pin(async { Ok(()) })
    }
}

//! `BaseEventBus` (spec 4.2): emit with schema check and payload
//! transformer, consumer group initialization, the consume loop (reclaim,
//! read, group by type, run listeners with a watchdog, ack / selective ack,
//! dead letters), stop and deregister.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use serde_json::Value;
use tokio::sync::Notify;

use crate::clock::Clock;
use crate::ctx::BoxError;
use crate::enqueue::BoxFuture;
use crate::events::config::EventBusConfig;
use crate::events::store::{DeadLetter, EventStore, EventStoreError, StreamEntry};
use crate::shutdown::CancelToken;

/// The payload key that carries the entry id (`EVENT_BUS_MESSAGE_ID`).
pub fn event_bus_message_id_key() -> &'static str {
    "__eventBusMessageId"
}

/// Bus errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventBusError {
    /// The store failed; the text is the cause.
    Store(String),
    /// Anything else, with its message.
    Other(String),
}

impl fmt::Display for EventBusError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EventBusError::Store(m) | EventBusError::Other(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for EventBusError {}

impl From<EventStoreError> for EventBusError {
    fn from(e: EventStoreError) -> EventBusError {
        EventBusError::Store(e.to_string())
    }
}

/// What a listener returns: the ids it failed (only used with selective
/// retry; those stay unacked).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListenerOutcome {
    pub failed_message_ids: Vec<String>,
}

/// A listener handler: the payloads of one batch of its event type (each
/// with the entry id under [`event_bus_message_id_key`]) and the batch
/// cancel token (the `AbortSignal`, cancelled on the watchdog timeout).
pub type ListenerFn = Arc<
    dyn Fn(Vec<Value>, CancelToken) -> BoxFuture<'static, Result<ListenerOutcome, BoxError>>
        + Send
        + Sync,
>;

/// `{ name, handler }`.
#[derive(Clone)]
pub struct EventListener {
    pub name: String,
    pub handler: ListenerFn,
}

/// Listeners by event type.
pub type Listeners = BTreeMap<String, Vec<EventListener>>;

/// `setPayloadHandler((eventType, payload) => payload)`.
pub type PayloadTransformer = Arc<dyn Fn(&str, Value) -> Value + Send + Sync>;

/// The wake-up signal of a stream in this process: `emit` notifies it, the
/// consume loop waits on it instead of a `BLOCK` poll (spec 4.2).
fn stream_signal(stream: &str) -> Arc<Notify> {
    static SIGNALS: Mutex<BTreeMap<String, Arc<Notify>>> = Mutex::new(BTreeMap::new());
    let mut map = SIGNALS.lock().unwrap_or_else(PoisonError::into_inner);
    Arc::clone(map.entry(stream.to_owned()).or_default())
}

#[derive(Default)]
struct ConsumerState {
    consumer: Option<String>,
    last_claim_at: Option<i64>,
    /// `lastProcessingErrors`: the last processing error per entry id.
    errors: HashMap<String, String>,
}

/// One parsed stream entry.
struct Message {
    id: String,
    event_type: String,
    payload: Value,
    raw: String,
    delivery_count: u32,
}

impl Message {
    fn of(entry: StreamEntry) -> Message {
        Message {
            payload: serde_json::from_str(&entry.payload).unwrap_or(Value::Null),
            id: entry.id,
            event_type: entry.event_type,
            raw: entry.payload,
            delivery_count: entry.delivery_count,
        }
    }
}

/// One bus over one stream and one consumer group.
pub struct EventBus {
    store: Arc<dyn EventStore>,
    config: EventBusConfig,
    clock: Arc<dyn Clock>,
    transformer: Option<PayloadTransformer>,
    initialized: AtomicBool,
    stop: Mutex<CancelToken>,
    state: Mutex<ConsumerState>,
}

impl EventBus {
    pub fn new(
        store: Arc<dyn EventStore>,
        config: EventBusConfig,
        clock: Arc<dyn Clock>,
    ) -> EventBus {
        EventBus {
            store,
            config,
            clock,
            transformer: None,
            initialized: AtomicBool::new(false),
            stop: Mutex::new(CancelToken::new()),
            state: Mutex::new(ConsumerState::default()),
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, ConsumerState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// `getConfig()`.
    pub fn config(&self) -> &EventBusConfig {
        &self.config
    }

    /// `setPayloadHandler(fn)`.
    pub fn with_payload_handler(mut self, transformer: PayloadTransformer) -> EventBus {
        self.transformer = Some(transformer);
        self
    }

    /// `cloneForGroup(group)`: same stream and options, another group.
    pub fn clone_for_group(&self, consumer_group: &str) -> EventBus {
        EventBus::new(
            Arc::clone(&self.store),
            EventBusConfig {
                consumer_group: consumer_group.to_owned(),
                ..self.config.clone()
            },
            Arc::clone(&self.clock),
        )
    }

    /// Creates the consumer group once; an existing group is not an error.
    pub async fn initialize(&self) -> Result<(), EventBusError> {
        if self.initialized.load(Ordering::SeqCst) {
            return Ok(());
        }
        match self
            .store
            .create_group(&self.config.stream_key, &self.config.consumer_group)
            .await
        {
            Ok(()) | Err(EventStoreError::BusyGroup) => {}
            Err(e) => return Err(e.into()),
        }
        self.initialized.store(true, Ordering::SeqCst);
        Ok(())
    }

    /// Schema check (invalid → `Ok("")`, nothing written), transformer,
    /// append with `max_len` and the clock time; returns the entry id.
    pub async fn emit(&self, event_type: &str, payload: Value) -> Result<String, EventBusError> {
        let payload = match &self.transformer {
            Some(transform) => transform(event_type, payload),
            None => payload,
        };
        if let Some(schema) = self.config.schemas.get(event_type)
            && !schema(&payload)
        {
            tracing::error!(
                event_type,
                stream = %self.config.stream_key,
                "[EventBus] Invalid payload"
            );
            return Ok(String::new());
        }
        let id = self
            .store
            .append(
                &self.config.stream_key,
                self.config.max_len,
                event_type,
                &payload.to_string(),
                self.clock.now_ms(),
            )
            .await?;
        stream_signal(&self.config.stream_key).notify_one();
        Ok(id)
    }

    /// One iteration of the consume loop (rate-limited reclaim, then one
    /// read of `read_batch_size`, then processing and ack).
    pub async fn consume_once(
        &self,
        consumer: &str,
        listeners: &Listeners,
    ) -> Result<(), EventBusError> {
        self.consume_iteration(consumer, listeners)
            .await
            .map(|_| ())
    }

    /// [`EventBus::consume_once`] returning how many new entries it read.
    async fn consume_iteration(
        &self,
        consumer: &str,
        listeners: &Listeners,
    ) -> Result<usize, EventBusError> {
        self.reclaim_stale(consumer, listeners).await;
        let entries = self
            .store
            .read_new(
                &self.config.stream_key,
                &self.config.consumer_group,
                consumer,
                self.config.read_batch_size,
            )
            .await?;
        let read = entries.len();
        if read > 0 {
            let messages = entries.into_iter().map(Message::of).collect();
            self.process_and_ack(messages, listeners).await?;
        }
        Ok(read)
    }

    /// `startConsuming(consumer, listeners)`: loops until [`EventBus::stop`];
    /// an iteration error is logged ("[EventBus] consume loop error") and
    /// retried after 1000 ms; when the loop ends the reader is closed.
    pub async fn start_consuming(&self, consumer: &str, listeners: Listeners) {
        let token = {
            let mut stop = self.stop.lock().unwrap_or_else(PoisonError::into_inner);
            if stop.is_cancelled() {
                *stop = CancelToken::new();
            }
            stop.clone()
        };
        self.state().consumer = Some(consumer.to_owned());
        let signal = stream_signal(&self.config.stream_key);
        while !token.is_cancelled() {
            let notified = signal.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            match self.consume_iteration(consumer, &listeners).await {
                Ok(0) => {
                    if token.is_cancelled() {
                        break;
                    }
                    let reclaim_wait = self.reclaim_wait(consumer).await;
                    tokio::select! {
                        () = &mut notified => {}
                        () = token.cancelled() => {}
                        () = sleep_opt(reclaim_wait) => {}
                    }
                }
                Ok(_) => {}
                Err(err) => {
                    tracing::error!(
                        err = %err,
                        stream = %self.config.stream_key,
                        "[EventBus] consume loop error"
                    );
                    if !token.is_cancelled() {
                        tokio::select! {
                            () = tokio::time::sleep(Duration::from_millis(1000)) => {}
                            () = token.cancelled() => {}
                        }
                    }
                }
            }
        }
        if self.store.close_reader(consumer).await.is_err() {
            self.store.force_close_reader(consumer);
        }
    }

    /// When idle: wake for the next reclaim only while entries of this
    /// consumer are unacked (no timer otherwise).
    async fn reclaim_wait(&self, consumer: &str) -> Option<Duration> {
        let interval = self.config.claim_interval_ms?;
        let pending = self
            .store
            .pending_count(
                &self.config.stream_key,
                &self.config.consumer_group,
                consumer,
            )
            .await
            .unwrap_or(1);
        (pending > 0).then(|| Duration::from_millis(interval.max(1000)))
    }

    /// The token that [`EventBus::stop`] cancels (for code that must stop
    /// the bus from elsewhere).
    pub fn stop_token(&self) -> CancelToken {
        self.stop
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Stops the consume loop (synchronous).
    pub fn stop(&self) {
        self.stop_token().cancel();
    }

    /// Removes the consumer when it has no unacked entries; otherwise keeps
    /// it and logs why.
    pub async fn deregister(&self) {
        let Some(consumer) = self.state().consumer.clone() else {
            return;
        };
        let stream = &self.config.stream_key;
        let group = &self.config.consumer_group;
        match self.store.pending_count(stream, group, &consumer).await {
            Ok(0) => {
                if let Err(err) = self.store.delete_consumer(stream, group, &consumer).await {
                    tracing::error!(err = %err, "[EventBus] DELCONSUMER failed");
                }
            }
            Ok(pending) => tracing::warn!(
                consumer_name = %consumer,
                pending,
                stream = %stream,
                "[EventBus] skipping DELCONSUMER because messages are still pending"
            ),
            Err(err) => tracing::error!(
                err = %err,
                stream = %stream,
                consumer_name = %consumer,
                "[EventBus] skipping DELCONSUMER after XPENDING failed"
            ),
        }
    }

    fn remember(&self, ids: &[String], error: &str) {
        let mut state = self.state();
        for id in ids {
            state.errors.insert(id.clone(), error.to_owned());
        }
    }

    fn forget(&self, ids: &[String]) {
        let mut state = self.state();
        for id in ids {
            state.errors.remove(id);
        }
    }

    /// `processEvents`: groups by type, runs every listener of a type on the
    /// whole batch concurrently; returns `(id, success)` in batch order.
    async fn process_events(
        &self,
        messages: &[Message],
        listeners: &Listeners,
        signal: CancelToken,
    ) -> Vec<(String, bool)> {
        let mut by_type: Vec<(&str, Vec<&Message>)> = Vec::new();
        for message in messages {
            match by_type.iter_mut().find(|(t, _)| *t == message.event_type) {
                Some((_, list)) => list.push(message),
                None => by_type.push((&message.event_type, vec![message])),
            }
        }
        let key = event_bus_message_id_key();
        let mut per_type: Vec<BoxFuture<'_, Vec<(String, bool)>>> = Vec::new();
        for (event_type, typed) in by_type {
            let applicable: Vec<&EventListener> = listeners
                .get(event_type)
                .map(|l| l.iter().collect())
                .unwrap_or_default();
            let ids: Vec<String> = typed.iter().map(|m| m.id.clone()).collect();
            if applicable.is_empty() {
                per_type.push(Box::pin(async move {
                    ids.into_iter().map(|id| (id, true)).collect()
                }));
                continue;
            }
            let payloads: Vec<Value> = typed
                .iter()
                .map(|m| match &m.payload {
                    Value::Object(object) => {
                        let mut object = object.clone();
                        object.insert(key.to_owned(), Value::String(m.id.clone()));
                        Value::Object(object)
                    }
                    other => other.clone(),
                })
                .collect();
            let signal = signal.clone();
            per_type.push(Box::pin(async move {
                let failed: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());
                let runs: Vec<BoxFuture<'_, ()>> = applicable
                    .into_iter()
                    .map(|listener| {
                        let payloads = payloads.clone();
                        let signal = signal.clone();
                        let ids = &ids;
                        let failed = &failed;
                        Box::pin(async move {
                            let count = payloads.len();
                            match (listener.handler)(payloads, signal).await {
                                Ok(outcome) => {
                                    let reported: Vec<String> = outcome
                                        .failed_message_ids
                                        .into_iter()
                                        .filter(|id| ids.contains(id))
                                        .collect();
                                    if !reported.is_empty() {
                                        self.remember(
                                            &reported,
                                            &format!(
                                                "listener {} reported failedMessageIds",
                                                listener.name
                                            ),
                                        );
                                        failed
                                            .lock()
                                            .unwrap_or_else(PoisonError::into_inner)
                                            .extend(reported);
                                    }
                                }
                                Err(err) => {
                                    self.remember(ids, &err.to_string());
                                    failed
                                        .lock()
                                        .unwrap_or_else(PoisonError::into_inner)
                                        .extend(ids.iter().cloned());
                                    tracing::error!(
                                        err = %err,
                                        stream_key = %self.config.stream_key,
                                        event_type,
                                        listener = %listener.name,
                                        count,
                                        "[EventBus] listener handler failed"
                                    );
                                }
                            }
                        }) as BoxFuture<'_, ()>
                    })
                    .collect();
                join_all(runs).await;
                let failed = failed.into_inner().unwrap_or_else(PoisonError::into_inner);
                ids.iter()
                    .map(|id| (id.clone(), !failed.contains(id)))
                    .collect()
            }));
        }
        let mut success: HashMap<String, bool> = HashMap::new();
        for results in join_all(per_type).await {
            success.extend(results);
        }
        messages
            .iter()
            .map(|m| (m.id.clone(), success.get(&m.id).copied().unwrap_or(true)))
            .collect()
    }

    /// `processAndAck`: runs the batch under the `process_timeout_ms`
    /// watchdog (timeout → cancel the token, leave the batch unacked), then
    /// acks every entry, or only the successful ones with selective retry.
    async fn process_and_ack(
        &self,
        messages: Vec<Message>,
        listeners: &Listeners,
    ) -> Result<(), EventBusError> {
        let timeout_ms = self.config.process_timeout_ms;
        let signal = CancelToken::new();
        let results = tokio::select! {
            results = self.process_events(&messages, listeners, signal.clone()) => results,
            () = tokio::time::sleep(Duration::from_millis(timeout_ms)) => {
                signal.cancel();
                let ids: Vec<String> = messages.iter().map(|m| m.id.clone()).collect();
                self.remember(&ids, &format!("processEvents timed out after {timeout_ms}ms"));
                tracing::warn!(
                    count = messages.len(),
                    stream = %self.config.stream_key,
                    timeout_ms,
                    "[EventBus] batch processing timed out; leaving unacked for reclaim"
                );
                return Ok(());
            }
        };
        let failed = results.iter().filter(|(_, ok)| !ok).count();
        let selective = self.config.enable_selective_retry == Some(true);
        if failed > 0 {
            if selective {
                tracing::warn!(
                    failed,
                    stream = %self.config.stream_key,
                    total = results.len(),
                    "[EventBus] leaving failed messages unacked for retry"
                );
            } else {
                tracing::warn!(
                    failed,
                    stream = %self.config.stream_key,
                    total = results.len(),
                    "[EventBus] acking batch despite handler failures"
                );
            }
        }
        let ack: Vec<String> = if selective {
            results
                .into_iter()
                .filter(|(_, ok)| *ok)
                .map(|(id, _)| id)
                .collect()
        } else {
            messages.iter().map(|m| m.id.clone()).collect()
        };
        if !ack.is_empty() {
            self.store
                .ack(
                    &self.config.stream_key,
                    &self.config.consumer_group,
                    ack.clone(),
                )
                .await?;
            self.forget(&ack);
        }
        Ok(())
    }

    /// `reclaimStaleEntries`: at most once per `claim_interval_ms`, claims
    /// entries idle longer than `claim_idle_ms`; past `max_deliveries` they
    /// go to the dead-letter stream, the rest are processed again. Errors
    /// are logged ("[EventBus] reclaim failed").
    async fn reclaim_stale(&self, consumer: &str, listeners: &Listeners) {
        let Some(interval) = self.config.claim_interval_ms else {
            return;
        };
        let now = self.clock.now_ms();
        {
            let mut state = self.state();
            if let Some(last) = state.last_claim_at
                && now.saturating_sub(last) < i64::try_from(interval).unwrap_or(i64::MAX)
            {
                return;
            }
            state.last_claim_at = Some(now);
        }
        if let Err(err) = self.reclaim_batch(consumer, listeners).await {
            tracing::error!(
                err = %err,
                stream = %self.config.stream_key,
                "[EventBus] reclaim failed"
            );
        }
    }

    async fn reclaim_batch(
        &self,
        consumer: &str,
        listeners: &Listeners,
    ) -> Result<(), EventBusError> {
        let claimed = self
            .store
            .reclaim(
                &self.config.stream_key,
                &self.config.consumer_group,
                consumer,
                self.config.claim_idle_ms,
                self.config.claim_batch_size,
            )
            .await?;
        let mut retryable = Vec::new();
        for message in claimed.into_iter().map(Message::of) {
            if message.delivery_count > self.config.max_deliveries {
                self.move_to_dead_letter(message).await?;
            } else {
                retryable.push(message);
            }
        }
        if !retryable.is_empty() {
            self.process_and_ack(retryable, listeners).await?;
        }
        Ok(())
    }

    async fn move_to_dead_letter(&self, message: Message) -> Result<(), EventBusError> {
        let error = self
            .state()
            .errors
            .get(&message.id)
            .cloned()
            .unwrap_or_else(|| "max deliveries exceeded".to_owned());
        let dead_letter_stream_key = self.config.dead_letter_stream_key.clone();
        self.store
            .dead_letter(DeadLetter {
                stream: dead_letter_stream_key.clone(),
                max_len: self.config.dead_letter_max_len,
                event_type: message.event_type.clone(),
                payload: message.raw.clone(),
                original_id: message.id.clone(),
                delivery_count: message.delivery_count,
                error,
                ts_ms: self.clock.now_ms(),
            })
            .await?;
        self.store
            .ack(
                &self.config.stream_key,
                &self.config.consumer_group,
                vec![message.id.clone()],
            )
            .await?;
        self.forget(std::slice::from_ref(&message.id));
        tracing::error!(
            dead_letter_stream_key = %dead_letter_stream_key,
            delivery_count = message.delivery_count,
            message_id = %message.id,
            stream = %self.config.stream_key,
            "[EventBus] moved message to dead-letter stream"
        );
        Ok(())
    }
}

async fn sleep_opt(wait: Option<Duration>) {
    match wait {
        Some(d) => tokio::time::sleep(d).await,
        None => std::future::pending().await,
    }
}

/// Polls every future to completion concurrently, results in input order
/// (no `futures` crate).
pub(crate) async fn join_all<'a, T>(futures: Vec<BoxFuture<'a, T>>) -> Vec<T> {
    let mut slots: Vec<(Option<BoxFuture<'a, T>>, Option<T>)> =
        futures.into_iter().map(|f| (Some(f), None)).collect();
    std::future::poll_fn(move |cx| {
        let mut done = true;
        for (future, out) in slots.iter_mut() {
            if let Some(f) = future {
                match f.as_mut().poll(cx) {
                    std::task::Poll::Ready(value) => {
                        *out = Some(value);
                        *future = None;
                    }
                    std::task::Poll::Pending => done = false,
                }
            }
        }
        if done {
            std::task::Poll::Ready(slots.iter_mut().filter_map(|(_, out)| out.take()).collect())
        } else {
            std::task::Poll::Pending
        }
    })
    .await
}

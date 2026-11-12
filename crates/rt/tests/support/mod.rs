//! Shared test support for the ported P2.8 frozen tests (spec 4.2). Test
//! code, not crate API: the fakes here are complete; the crate code under
//! test is not.

#![allow(dead_code)]

pub mod gateway;

use std::collections::{BTreeMap, VecDeque};
use std::fmt;
use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chatbotx_db::Db;
use chatbotx_rt::delivery::{
    GuestSocket, GuestSocketData, MemberSocket, MemberSocketData, SendStatus, SocketSink,
};
use chatbotx_rt::presence::PresenceSink;
use chatbotx_rt::revoke::{RevokeArgs, RevokeRetry, RevokeStore};
use chatbotx_rt::token::ChatScope;
use chatbotx_rt::{BoxError, BoxFuture};
use hmac::{KeyInit, Mac};
use serde_json::Value;
use tokio::sync::Notify;
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt};

// ---------------------------------------------------------------------------
// HS256 signing of raw claims (replaces `jose` `new SignJWT(..)` in tests)
// ---------------------------------------------------------------------------

/// Signs `claims` as-is with header `{"alg":"HS256"}`; independent of the
/// crate's own signer.
pub fn sign_raw_jwt(claims: &Value, secret: &str) -> String {
    let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"HS256"}"#);
    let payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(claims).unwrap());
    let signing_input = format!("{header}.{payload}");
    let mut mac = <hmac::Hmac<sha2::Sha256> as KeyInit>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(signing_input.as_bytes());
    let signature = URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());
    format!("{signing_input}.{signature}")
}

/// Decodes the (unverified) payload segment of a JWT.
pub fn decode_jwt_payload(token: &str) -> Value {
    let payload = token.split('.').nth(1).unwrap();
    serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload).unwrap()).unwrap()
}

/// `Math.floor(Date.now() / 1000)` for a ms clock value.
pub fn secs(now_ms: i64) -> i64 {
    now_ms.div_euclid(1000)
}

// ---------------------------------------------------------------------------
// FakeSink: the TS fake socket (`sent[]`, `closed`, `end`, `send` -> 1/2/throw)
// ---------------------------------------------------------------------------

/// What `send` does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SendMode {
    /// Records the frame, returns `Sent` (uWS `1`).
    Ok,
    /// Records nothing, returns `Dropped` (uWS `2`).
    Dropped,
    /// Returns an error (the TS `send` throws `socket write failed`).
    Fail,
}

#[derive(Debug)]
pub struct WriteFailed;

impl fmt::Display for WriteFailed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("socket write failed")
    }
}

impl std::error::Error for WriteFailed {}

/// A recording socket sink.
pub struct FakeSink {
    mode: Mutex<SendMode>,
    sent: Mutex<Vec<String>>,
    ends: Mutex<Vec<(u16, String)>>,
    send_calls: AtomicUsize,
}

impl FakeSink {
    pub fn new(mode: SendMode) -> Arc<FakeSink> {
        Arc::new(FakeSink {
            mode: Mutex::new(mode),
            sent: Mutex::new(Vec::new()),
            ends: Mutex::new(Vec::new()),
            send_calls: AtomicUsize::new(0),
        })
    }

    pub fn set_mode(&self, mode: SendMode) {
        *self.mode.lock().unwrap() = mode;
    }

    /// Frames sent with `SendMode::Ok`.
    pub fn sent(&self) -> Vec<String> {
        self.sent.lock().unwrap().clone()
    }

    /// Every `end(code, reason)` call.
    pub fn ends(&self) -> Vec<(u16, String)> {
        self.ends.lock().unwrap().clone()
    }

    /// The first close, as the TS `socket.closed` (`null` -> `None`).
    pub fn closed(&self) -> Option<(u16, String)> {
        self.ends.lock().unwrap().first().cloned()
    }

    pub fn send_calls(&self) -> usize {
        self.send_calls.load(Ordering::SeqCst)
    }
}

impl SocketSink for FakeSink {
    fn send(&self, frame: &str) -> Result<SendStatus, BoxError> {
        self.send_calls.fetch_add(1, Ordering::SeqCst);
        match *self.mode.lock().unwrap() {
            SendMode::Ok => {
                self.sent.lock().unwrap().push(frame.to_owned());
                Ok(SendStatus::Sent)
            }
            SendMode::Dropped => Ok(SendStatus::Dropped),
            SendMode::Fail => Err(Box::new(WriteFailed)),
        }
    }

    fn end(&self, code: u16, reason: &str) {
        self.ends.lock().unwrap().push((code, reason.to_owned()));
    }
}

/// The TS `createSocket` defaults: scope `all`, user `user-1`,
/// `workspace-1`, no teams, `iatMs` 0.
pub fn member_data() -> MemberSocketData {
    MemberSocketData {
        workspace_id: "workspace-1".to_owned(),
        user_id: "user-1".to_owned(),
        chat_scope: ChatScope::All,
        team_ids: Vec::new(),
        iat_ms: 0,
    }
}

/// A member socket over a fresh `FakeSink`.
pub fn member_socket(data: MemberSocketData, mode: SendMode) -> (Arc<MemberSocket>, Arc<FakeSink>) {
    let sink = FakeSink::new(mode);
    (MemberSocket::new(data, sink.clone()), sink)
}

/// A guest socket over a fresh `FakeSink`.
pub fn guest_socket(
    workspace_id: &str,
    guest_conversation_id: &str,
    mode: SendMode,
) -> (Arc<GuestSocket>, Arc<FakeSink>) {
    let sink = FakeSink::new(mode);
    let socket = GuestSocket::new(
        GuestSocketData {
            workspace_id: workspace_id.to_owned(),
            guest_conversation_id: guest_conversation_id.to_owned(),
        },
        sink.clone(),
    );
    (socket, sink)
}

pub fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| (*s).to_owned()).collect()
}

// ---------------------------------------------------------------------------
// Deferred: `Promise.withResolvers<void>()`
// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct Deferred {
    resolved: AtomicBool,
    notify: Notify,
}

impl Deferred {
    pub fn new() -> Arc<Deferred> {
        Arc::new(Deferred::default())
    }

    pub fn resolve(&self) {
        self.resolved.store(true, Ordering::SeqCst);
        self.notify.notify_waiters();
    }

    pub fn is_resolved(&self) -> bool {
        self.resolved.load(Ordering::SeqCst)
    }

    pub async fn wait(&self) {
        loop {
            let notified = self.notify.notified();
            if self.is_resolved() {
                return;
            }
            notified.await;
        }
    }
}

// ---------------------------------------------------------------------------
// Waiting
// ---------------------------------------------------------------------------

/// `flushMicrotasks`: lets spawned tasks run a few steps.
pub async fn flush_tasks() {
    for _ in 0..16 {
        tokio::task::yield_now().await;
    }
}

/// `vi.waitFor`: polls `condition` (yielding, then sleeping 1 ms) until it
/// holds; panics after `timeout`.
pub async fn wait_for(timeout: Duration, mut condition: impl FnMut() -> bool) {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if condition() {
            return;
        }
        flush_tasks().await;
        if condition() {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "condition not met within {timeout:?}"
        );
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
}

/// Real-time wait for gateway tests (`IO_WAIT` = 15 s).
pub const IO_WAIT: Duration = Duration::from_secs(15);

/// Runs `future` with a 30 s timeout (the vitest test timeout).
pub async fn within_test_timeout<T>(future: impl Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(30), future)
        .await
        .expect("test timed out")
}

// ---------------------------------------------------------------------------
// FakeRevokeStore: the fake Redis `get` / `markRealtimeMemberRevoked`
// ---------------------------------------------------------------------------

/// One scripted marker read.
pub enum ReadStep {
    /// Resolves with this marker value.
    Value(Option<String>),
    /// Fails (`Redis unavailable`).
    Fail,
    /// Signals `started`, then waits for `release`, then resolves `None`.
    Block {
        started: Arc<Deferred>,
        release: Arc<Deferred>,
    },
}

#[derive(Debug)]
pub struct StoreUnavailable;

impl fmt::Display for StoreUnavailable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Redis unavailable")
    }
}

impl std::error::Error for StoreUnavailable {}

/// Scripted reads (then `default_value`), recorded raises.
#[derive(Default)]
pub struct FakeRevokeStore {
    script: Mutex<VecDeque<ReadStep>>,
    default_value: Mutex<Option<String>>,
    reads: AtomicUsize,
    raises: Mutex<Vec<(String, String, u64)>>,
    raise_failures_left: Mutex<usize>,
    raise_always_fails: AtomicBool,
    on_raise: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
}

impl FakeRevokeStore {
    pub fn new() -> Arc<FakeRevokeStore> {
        Arc::new(FakeRevokeStore::default())
    }

    /// Every unscripted read resolves `value` (the TS `mockResolvedValue`).
    pub fn set_default(&self, value: Option<String>) {
        *self.default_value.lock().unwrap() = value;
    }

    /// The next read does `step` (the TS `mock*Once`).
    pub fn push_read(&self, step: ReadStep) {
        self.script.lock().unwrap().push_back(step);
    }

    pub fn reads(&self) -> usize {
        self.reads.load(Ordering::SeqCst)
    }

    /// The next `n` raises fail.
    pub fn fail_raises(&self, n: usize) {
        *self.raise_failures_left.lock().unwrap() = n;
    }

    /// Every raise fails.
    pub fn fail_every_raise(&self) {
        self.raise_always_fails.store(true, Ordering::SeqCst);
    }

    /// Runs `hook` inside every raise call (to observe what happened first).
    pub fn set_on_raise(&self, hook: Arc<dyn Fn() + Send + Sync>) {
        *self.on_raise.lock().unwrap() = Some(hook);
    }

    /// Every `(workspace_id, user_id, revoked_at_ms)` raise call.
    pub fn raises(&self) -> Vec<(String, String, u64)> {
        self.raises.lock().unwrap().clone()
    }
}

impl RevokeStore for FakeRevokeStore {
    fn read_marker<'a>(
        &'a self,
        _workspace_id: &'a str,
        _user_id: &'a str,
    ) -> BoxFuture<'a, Result<Option<String>, BoxError>> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        let step = self.script.lock().unwrap().pop_front();
        let default = self.default_value.lock().unwrap().clone();
        Box::pin(async move {
            match step {
                None => Ok(default),
                Some(ReadStep::Value(value)) => Ok(value),
                Some(ReadStep::Fail) => Err(Box::new(StoreUnavailable) as BoxError),
                Some(ReadStep::Block { started, release }) => {
                    started.resolve();
                    release.wait().await;
                    Ok(None)
                }
            }
        })
    }

    fn raise_marker<'a>(
        &'a self,
        workspace_id: &'a str,
        user_id: &'a str,
        revoked_at_ms: u64,
    ) -> BoxFuture<'a, Result<(), BoxError>> {
        self.raises.lock().unwrap().push((
            workspace_id.to_owned(),
            user_id.to_owned(),
            revoked_at_ms,
        ));
        if let Some(hook) = self.on_raise.lock().unwrap().clone() {
            hook();
        }
        let fail = self.raise_always_fails.load(Ordering::SeqCst) || {
            let mut left = self.raise_failures_left.lock().unwrap();
            let fail = *left > 0;
            *left = left.saturating_sub(1);
            fail
        };
        Box::pin(async move {
            if fail {
                Err(Box::new(StoreUnavailable) as BoxError)
            } else {
                Ok(())
            }
        })
    }
}

// ---------------------------------------------------------------------------
// RecordingPresence: the `reportWorkspacePresence` mock
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct ReportFailed;

impl fmt::Display for ReportFailed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("network error")
    }
}

impl std::error::Error for ReportFailed {}

#[derive(Default)]
pub struct RecordingPresence {
    calls: Mutex<Vec<(String, Vec<String>)>>,
    fail: AtomicBool,
}

impl RecordingPresence {
    pub fn new() -> Arc<RecordingPresence> {
        Arc::new(RecordingPresence::default())
    }

    pub fn failing() -> Arc<RecordingPresence> {
        let sink = RecordingPresence::default();
        sink.fail.store(true, Ordering::SeqCst);
        Arc::new(sink)
    }

    pub fn calls(&self) -> Vec<(String, Vec<String>)> {
        self.calls.lock().unwrap().clone()
    }

    pub fn calls_for(&self, workspace_id: &str) -> Vec<Vec<String>> {
        self.calls()
            .into_iter()
            .filter(|(ws, _)| ws == workspace_id)
            .map(|(_, ids)| ids)
            .collect()
    }

    /// `mockClear()`.
    pub fn clear(&self) {
        self.calls.lock().unwrap().clear();
    }
}

impl PresenceSink for RecordingPresence {
    fn report<'a>(
        &'a self,
        workspace_id: &'a str,
        user_ids: &'a [String],
    ) -> BoxFuture<'a, Result<(), BoxError>> {
        self.calls
            .lock()
            .unwrap()
            .push((workspace_id.to_owned(), user_ids.to_vec()));
        let fail = self.fail.load(Ordering::SeqCst);
        Box::pin(async move {
            if fail {
                Err(Box::new(ReportFailed) as BoxError)
            } else {
                Ok(())
            }
        })
    }
}

// ---------------------------------------------------------------------------
// RecordingRevokeRetry: the `defaultQueue.add` mock
// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct RecordingRevokeRetry {
    calls: Mutex<Vec<RevokeArgs>>,
}

impl RecordingRevokeRetry {
    pub fn new() -> Arc<RecordingRevokeRetry> {
        Arc::new(RecordingRevokeRetry::default())
    }

    pub fn calls(&self) -> Vec<RevokeArgs> {
        self.calls.lock().unwrap().clone()
    }
}

impl RevokeRetry for RecordingRevokeRetry {
    fn enqueue<'a>(&'a self, args: &'a RevokeArgs) -> BoxFuture<'a, Result<(), BoxError>> {
        self.calls.lock().unwrap().push(args.clone());
        Box::pin(async { Ok(()) })
    }
}

// ---------------------------------------------------------------------------
// RecordingLog: keeps tracing events (replaces logger spies)
// ---------------------------------------------------------------------------

/// One captured `tracing` event. Field names are the Rust names of the TS
/// log object keys (`workspaceId` -> `workspace_id`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogEntry {
    pub level: Level,
    pub message: String,
    pub fields: BTreeMap<String, String>,
}

impl LogEntry {
    pub fn field(&self, name: &str) -> Option<&str> {
        self.fields.get(name).map(String::as_str)
    }
}

/// Captures every `tracing` event of the current thread while it lives.
/// Use with `#[test]` or the default current-thread `#[tokio::test]`.
pub struct RecordingLog {
    entries: Arc<Mutex<Vec<LogEntry>>>,
    _guard: tracing::subscriber::DefaultGuard,
}

impl RecordingLog {
    pub fn install() -> RecordingLog {
        let entries = Arc::new(Mutex::new(Vec::new()));
        let layer = CaptureLayer {
            entries: Arc::clone(&entries),
        };
        let subscriber = tracing_subscriber::registry().with(layer);
        let guard = tracing::subscriber::set_default(subscriber);
        RecordingLog {
            entries,
            _guard: guard,
        }
    }

    pub fn entries(&self) -> Vec<LogEntry> {
        self.entries.lock().unwrap().clone()
    }

    pub fn at(&self, level: Level) -> Vec<LogEntry> {
        self.entries()
            .into_iter()
            .filter(|e| e.level == level)
            .collect()
    }

    /// Entries with exactly this message.
    pub fn with_message(&self, message: &str) -> Vec<LogEntry> {
        self.entries()
            .into_iter()
            .filter(|e| e.message == message)
            .collect()
    }

    pub fn find(&self, message: &str) -> Option<LogEntry> {
        self.entries().into_iter().find(|e| e.message == message)
    }

    pub fn clear(&self) {
        self.entries.lock().unwrap().clear();
    }
}

struct CaptureLayer {
    entries: Arc<Mutex<Vec<LogEntry>>>,
}

#[derive(Default)]
struct FieldVisitor {
    message: String,
    fields: BTreeMap<String, String>,
}

impl FieldVisitor {
    fn put(&mut self, field: &Field, value: String) {
        if field.name() == "message" {
            self.message = value;
        } else {
            self.fields.insert(field.name().to_owned(), value);
        }
    }
}

impl Visit for FieldVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.put(field, value.to_owned());
    }
    fn record_i64(&mut self, field: &Field, value: i64) {
        self.put(field, value.to_string());
    }
    fn record_u64(&mut self, field: &Field, value: u64) {
        self.put(field, value.to_string());
    }
    fn record_bool(&mut self, field: &Field, value: bool) {
        self.put(field, value.to_string());
    }
    fn record_f64(&mut self, field: &Field, value: f64) {
        self.put(field, value.to_string());
    }
    fn record_error(&mut self, field: &Field, value: &(dyn std::error::Error + 'static)) {
        self.put(field, value.to_string());
    }
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.put(field, format!("{value:?}"));
    }
}

impl<S: Subscriber> Layer<S> for CaptureLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut visitor = FieldVisitor::default();
        event.record(&mut visitor);
        self.entries.lock().unwrap().push(LogEntry {
            level: *event.metadata().level(),
            message: visitor.message,
            fields: visitor.fields,
        });
    }

    fn on_new_span(&self, _attrs: &Attributes<'_>, _id: &Id, _ctx: Context<'_, S>) {}

    fn on_record(&self, _span: &Id, _values: &Record<'_>, _ctx: Context<'_, S>) {}
}

// ---------------------------------------------------------------------------
// A Realtime with fakes
// ---------------------------------------------------------------------------

/// The fakes behind a test `Realtime`.
#[derive(Clone)]
pub struct Fakes {
    pub revoke_store: Arc<FakeRevokeStore>,
    pub presence: Arc<RecordingPresence>,
    pub revoke_retry: Arc<RecordingRevokeRetry>,
}

impl Fakes {
    pub fn new() -> Fakes {
        Fakes {
            revoke_store: FakeRevokeStore::new(),
            presence: RecordingPresence::new(),
            revoke_retry: RecordingRevokeRetry::new(),
        }
    }

    pub fn deps(&self, clock: Arc<dyn chatbotx_rt::Clock>) -> chatbotx_rt::RealtimeDeps {
        chatbotx_rt::RealtimeDeps {
            clock,
            revoke_store: self.revoke_store.clone(),
            presence: self.presence.clone(),
            revoke_retry: self.revoke_retry.clone(),
        }
    }
}

/// The TS test environment: `REALTIME_BROADCAST_SECRET` of the helper,
/// `REALTIME_SHUTDOWN_DRAIN_MS=0`, the other defaults.
pub fn test_config(secret: &str) -> chatbotx_rt::RtConfig {
    let mut cfg = chatbotx_rt::RtConfig::new(secret);
    cfg.shutdown_drain_ms = 0;
    cfg
}

/// A `Realtime` on `clock` with fresh fakes. Must run inside a runtime.
pub fn test_realtime(clock: Arc<dyn chatbotx_rt::Clock>) -> (chatbotx_rt::Realtime, Fakes) {
    let fakes = Fakes::new();
    let rt = chatbotx_rt::Realtime::with_deps(
        test_config("gateway-test-secretgateway-test-secret"),
        fakes.deps(clock),
    );
    (rt, fakes)
}

// ---------------------------------------------------------------------------
// Database
// ---------------------------------------------------------------------------

/// A private in-memory database with the migrations applied.
pub async fn migrated_db() -> Arc<Db> {
    let db = Db::open_in_memory().expect("open in-memory db");
    db.migrate().await.expect("migrate");
    Arc::new(db)
}

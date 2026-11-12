//! The WebSocket gateway (spec 1.1-1.2, 2.4-2.5, 3.4): axum routes
//! `/rt/workspaces/{workspace_id}`, `/rt/guests/{guest_conversation_id}`,
//! `/health`, `/ready`; connect auth, admission, revoke check, replay,
//! heartbeats, lifetime and the slow-consumer rule.
//!
//! One connection = a main task (reader half, live `broadcast` receiver,
//! heartbeat signal, lifetime sleep) and a writer task fed by an outbound
//! queue with a 512 000-byte budget (the uWS `maxBackpressure`). Admission
//! slots, the workspace reference and the hub registration are RAII guards:
//! an aborted upgrade (axum drops the handler future or the upgrade fails)
//! releases them.

use std::collections::hash_map::RandomState;
use std::collections::{HashMap, HashSet};
use std::hash::{BuildHasher, Hasher};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::extract::ws::{CloseFrame, Message, Utf8Bytes, WebSocket, WebSocketUpgrade};
use axum::http::{StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use futures_util::stream::SplitStream;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::{Notify, broadcast, mpsc, watch};
use tokio::task::JoinHandle;
use tokio::time::Instant;

use crate::BoxError;
use crate::delivery::{
    Delivery, GuestSocket, GuestSocketData, MemberSocket, MemberSocketData, SendStatus, SocketSink,
};
use crate::hub::{Attached, Entry, WorkspaceRef};
use crate::presence::{
    PresenceReporter, PresenceReporterOptions, create_presence_reporter, report_workspace_presence,
};
use crate::protocol::{HEARTBEAT_FRAME, close, reason, retry_after_reason};
use crate::publish::{Inner, Realtime, RealtimeDeps};
use crate::token::{MemberClaims, verify_guest_connect_token, verify_member_connect_token};

/// The overload `retryAfter` range: `1000 + rand(4000)` ms.
pub const OVERLOAD_RETRY_AFTER_MIN_MS: u64 = 1000;
pub const OVERLOAD_RETRY_AFTER_SPREAD_MS: u64 = 4000;
/// The uWS `maxBackpressure`: pending outbound bytes of one socket.
pub const SLOW_CONSUMER_BUFFER_BYTES: usize = 512_000;
/// `CONNECTION_LIFETIME_JITTER_RATIO`.
pub const CONNECTION_LIFETIME_JITTER_RATIO: f64 = 0.15;
/// `PRESENCE_REPORT_CONCURRENCY`.
pub const PRESENCE_REPORT_CONCURRENCY: usize = 16;
/// `TOKEN_REJECTION_LOG_WINDOW_MS`.
const TOKEN_REJECTION_LOG_WINDOW_MS: i64 = 10_000;
/// A single write that does not finish in this time drops the connection.
const WRITE_TIMEOUT: Duration = Duration::from_secs(30);
/// Time for a close frame (and the peer's close reply).
const CLOSE_TIMEOUT: Duration = Duration::from_secs(2);
/// Clients send nothing but pongs and closes.
const MAX_CLIENT_MESSAGE_BYTES: usize = 64 * 1024;

/// The router the server merges.
pub fn router(rt: Realtime) -> Router {
    Router::new()
        .route("/rt/workspaces/{workspace_id}", get(member_upgrade))
        .route("/rt/guests/{guest_conversation_id}", get(guest_upgrade))
        .route("/health", get(health))
        .route("/ready", get(ready))
        .with_state(rt)
}

/// The result of the open-time steps of one socket.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenOutcome {
    /// A step closed the socket: no presence, no lifetime timer.
    Closed,
    /// Open; the lifetime timer fires after `lifetime_ms`.
    Open { lifetime_ms: u64 },
}

static RANDOM_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A uniform value in `[0, 1)` (no `rand` dependency).
fn random_unit() -> f64 {
    let mut hasher = RandomState::new().build_hasher();
    hasher.write_u64(RANDOM_COUNTER.fetch_add(1, Ordering::Relaxed));
    (hasher.finish() >> 11) as f64 / (1_u64 << 53) as f64
}

/// A jittered retry hint for an overloaded close.
pub fn next_overload_retry_after_ms() -> u64 {
    OVERLOAD_RETRY_AFTER_MIN_MS + (random_unit() * OVERLOAD_RETRY_AFTER_SPREAD_MS as f64) as u64
}

// ---------------------------------------------------------------------------
// Gateway state
// ---------------------------------------------------------------------------

/// Admission counts (`GatewayConnectionState`): open sockets and pending
/// upgrades, global and per key.
#[derive(Default)]
struct Admission {
    active: usize,
    active_guests: usize,
    pending: usize,
    pending_guests: usize,
    peak: usize,
    member_by_workspace: HashMap<String, usize>,
    member_pending_by_workspace: HashMap<String, usize>,
    guest_by_key: HashMap<(String, String), usize>,
    guest_pending_by_key: HashMap<(String, String), usize>,
    guest_by_workspace: HashMap<String, usize>,
    guest_pending_by_workspace: HashMap<String, usize>,
}

fn count<K: Eq + std::hash::Hash + Clone>(map: &HashMap<K, usize>, key: &K) -> usize {
    map.get(key).copied().unwrap_or(0)
}

fn increment<K: Eq + std::hash::Hash + Clone>(map: &mut HashMap<K, usize>, key: &K) {
    *map.entry(key.clone()).or_default() += 1;
}

/// `decrementKeyedCount`: deletes the key at zero.
fn decrement<K: Eq + std::hash::Hash + Clone>(map: &mut HashMap<K, usize>, key: &K) {
    if let Some(n) = map.get_mut(key) {
        *n = n.saturating_sub(1);
        if *n == 0 {
            map.remove(key);
        }
    }
}

struct PresenceState {
    users: Mutex<HashMap<String, HashSet<String>>>,
    /// ws -> (generation, coalesce timer).
    timers: Mutex<HashMap<String, (u64, JoinHandle<()>)>>,
    next_timer: AtomicU64,
    reporter: PresenceReporter,
    sweep: Mutex<Option<JoinHandle<()>>>,
}

#[derive(Default)]
struct Heartbeat {
    open: usize,
    task: Option<JoinHandle<()>>,
}

#[derive(Default)]
struct TokenLog {
    last_at: Option<i64>,
    suppressed: u64,
}

/// The gateway part of [`Inner`].
pub(crate) struct GatewayState {
    this: Weak<Inner>,
    stopping: AtomicBool,
    admission: Mutex<Admission>,
    presence: PresenceState,
    heartbeat: Mutex<Heartbeat>,
    heartbeat_tx: watch::Sender<u64>,
    token_log: Mutex<TokenLog>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl GatewayState {
    pub(crate) fn new(this: Weak<Inner>, deps: &RealtimeDeps) -> GatewayState {
        let connected = this.clone();
        let user_ids = this.clone();
        let clock = Arc::clone(&deps.clock);
        let sink = Arc::clone(&deps.presence);
        let reporter = create_presence_reporter(PresenceReporterOptions {
            concurrency: PRESENCE_REPORT_CONCURRENCY,
            get_connected_workspace_ids: Arc::new(move || {
                connected.upgrade().map_or_else(Vec::new, |inner| {
                    lock(&inner.gateway.presence.users)
                        .keys()
                        .cloned()
                        .collect()
                })
            }),
            get_user_ids: Arc::new(move |workspace_id| {
                user_ids.upgrade().map_or_else(Vec::new, |inner| {
                    lock(&inner.gateway.presence.users)
                        .get(workspace_id)
                        .map(|users| users.iter().cloned().collect())
                        .unwrap_or_default()
                })
            }),
            now: Some(Arc::new(move || clock.now_ms())),
            on_error: Some(Arc::new(|err: &BoxError, workspace_id: &str| {
                tracing::error!(
                    err = %err,
                    workspace_id = %workspace_id,
                    "Failed to report workspace presence"
                );
            })),
            report: Arc::new(move |workspace_id, user_ids| {
                let sink = Arc::clone(&sink);
                Box::pin(async move {
                    Ok(report_workspace_presence(&*sink, &workspace_id, &user_ids).await)
                })
            }),
        });
        GatewayState {
            this,
            stopping: AtomicBool::new(false),
            admission: Mutex::new(Admission::default()),
            presence: PresenceState {
                users: Mutex::new(HashMap::new()),
                timers: Mutex::new(HashMap::new()),
                next_timer: AtomicU64::new(0),
                reporter,
                sweep: Mutex::new(None),
            },
            heartbeat: Mutex::new(Heartbeat::default()),
            heartbeat_tx: watch::channel(0).0,
            token_log: Mutex::new(TokenLog::default()),
        }
    }

    pub(crate) fn is_stopping(&self) -> bool {
        self.stopping.load(Ordering::SeqCst)
    }

    /// `true` for the first call only.
    pub(crate) fn begin_shutdown(&self) -> bool {
        !self.stopping.swap(true, Ordering::SeqCst)
    }

    /// Stops the reporter, the sweep and every coalesce timer.
    pub(crate) fn stop_presence(&self) {
        self.presence.reporter.stop();
        if let Some(sweep) = lock(&self.presence.sweep).take() {
            sweep.abort();
        }
        for (_, (_, timer)) in lock(&self.presence.timers).drain() {
            timer.abort();
        }
    }

    /// `realtime_server_metric`: one window of the gateway counters.
    pub(crate) fn log_metrics(&self, inner: &Inner) {
        let counters = inner.hub.delivery().counters().take();
        let (connections, peak) = {
            let mut admission = lock(&self.admission);
            let peak = admission.peak;
            admission.peak = admission.active;
            (admission.active, peak)
        };
        tracing::info!(
            metric = "realtime_server",
            connections,
            max_connections = peak,
            drops = counters.drops,
            overload_closes = counters.overload_closes,
            records = counters.records,
            send_bytes = counters.send_bytes,
            send_failures = counters.send_failures,
            sends = counters.sends,
            token_rejections = counters.token_rejections,
            upgrades = counters.upgrades,
            "realtime_server_metric"
        );
    }

    // ---- presence ----

    /// Adds the user of a freshly opened member socket; a new user marks
    /// the workspace dirty (coalesced report).
    fn presence_add(&self, workspace_id: &str, user_id: &str) {
        let (newly, first_workspace) = {
            let mut users = lock(&self.presence.users);
            let first_workspace = users.is_empty();
            let newly = users
                .entry(workspace_id.to_owned())
                .or_default()
                .insert(user_id.to_owned());
            (newly, first_workspace)
        };
        if first_workspace {
            self.start_presence_sweep();
        }
        if newly {
            self.mark_presence_dirty(workspace_id);
        }
    }

    /// The user's last socket closed: no report (the lease expires).
    fn presence_remove(&self, workspace_id: &str, user_id: &str) {
        let none_left = {
            let mut users = lock(&self.presence.users);
            if let Some(set) = users.get_mut(workspace_id) {
                set.remove(user_id);
                if set.is_empty() {
                    users.remove(workspace_id);
                }
            }
            users.is_empty()
        };
        if none_left {
            if let Some(sweep) = lock(&self.presence.sweep).take() {
                sweep.abort();
            }
        }
    }

    fn mark_presence_dirty(&self, workspace_id: &str) {
        if self.is_stopping() {
            return;
        }
        let Some(inner) = self.this.upgrade() else {
            return;
        };
        let generation = self.presence.next_timer.fetch_add(1, Ordering::Relaxed);
        let weak = self.this.clone();
        let workspace = workspace_id.to_owned();
        let delay = Duration::from_millis(inner.cfg.presence_report_coalesce_ms);
        let mut timers = lock(&self.presence.timers);
        if let Some((_, previous)) = timers.remove(workspace_id) {
            previous.abort();
        }
        let timer = tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            let Some(inner) = weak.upgrade() else {
                return;
            };
            let fire = {
                let mut timers = lock(&inner.gateway.presence.timers);
                let current = timers
                    .get(&workspace)
                    .is_some_and(|(timer_generation, _)| *timer_generation == generation);
                if current {
                    timers.remove(&workspace);
                }
                current
            };
            if fire {
                inner.gateway.presence.reporter.report_now(&workspace);
            }
        });
        timers.insert(workspace_id.to_owned(), (generation, timer));
    }

    /// The periodic sweep: only while some member socket is open.
    fn start_presence_sweep(&self) {
        let Some(inner) = self.this.upgrade() else {
            return;
        };
        let mut sweep = lock(&self.presence.sweep);
        if sweep.is_some() || self.is_stopping() {
            return;
        }
        let period = Duration::from_millis(inner.cfg.presence_report_interval_ms.max(1));
        let weak = self.this.clone();
        *sweep = Some(tokio::spawn(async move {
            let mut interval = tokio::time::interval_at(Instant::now() + period, period);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                interval.tick().await;
                let Some(inner) = weak.upgrade() else {
                    return;
                };
                inner.gateway.presence.reporter.sweep();
            }
        }));
    }

    // ---- heartbeat ----

    /// One more open socket; the first starts the shared heartbeat.
    fn socket_opened(&self) {
        let Some(inner) = self.this.upgrade() else {
            return;
        };
        let mut heartbeat = lock(&self.heartbeat);
        heartbeat.open += 1;
        if heartbeat.task.is_some() {
            return;
        }
        let period = Duration::from_millis(inner.cfg.heartbeat_interval_ms.max(1));
        let weak = self.this.clone();
        heartbeat.task = Some(tokio::spawn(async move {
            let mut interval = tokio::time::interval_at(Instant::now() + period, period);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                interval.tick().await;
                let Some(inner) = weak.upgrade() else {
                    return;
                };
                inner.gateway.heartbeat_tx.send_modify(|tick| *tick += 1);
                inner.gateway.log_metrics(&inner);
            }
        }));
    }

    /// One socket fewer; the last stops the heartbeat.
    fn socket_closed(&self) {
        let mut heartbeat = lock(&self.heartbeat);
        heartbeat.open = heartbeat.open.saturating_sub(1);
        if heartbeat.open == 0 {
            if let Some(task) = heartbeat.task.take() {
                task.abort();
            }
        }
    }

    /// `logTokenRejection`: at most one warn per 10 s, with the count of
    /// suppressed rejections.
    fn log_token_rejection(&self, now_ms: i64, route: Route<'_>, err: &dyn std::fmt::Display) {
        let suppressed = {
            let mut log = lock(&self.token_log);
            if log
                .last_at
                .is_some_and(|last| now_ms - last < TOKEN_REJECTION_LOG_WINDOW_MS)
            {
                log.suppressed += 1;
                return;
            }
            log.last_at = Some(now_ms);
            std::mem::take(&mut log.suppressed)
        };
        match route {
            Route::Workspace(workspace_id) => tracing::warn!(
                err = %err,
                suppressed,
                workspace_id = %workspace_id,
                "Realtime connect token verification failed"
            ),
            Route::Guest(guest_conversation_id) => tracing::warn!(
                err = %err,
                suppressed,
                guest_conversation_id = %guest_conversation_id,
                "Realtime connect token verification failed"
            ),
        }
    }
}

#[derive(Clone, Copy)]
enum Route<'a> {
    Workspace(&'a str),
    Guest(&'a str),
}

// ---------------------------------------------------------------------------
// Admission guards
// ---------------------------------------------------------------------------

enum Slot {
    Member {
        workspace_id: String,
    },
    Guest {
        workspace_id: String,
        guest_conversation_id: String,
    },
    /// An overloaded upgrade: counts as pending until it opens.
    Overloaded {
        guest: bool,
    },
}

#[derive(PartialEq, Eq)]
enum Stage {
    Pending,
    Active,
}

/// `createPendingUpgrade` + the open/close slot bookkeeping. Dropped while
/// pending (aborted upgrade) or active (closed socket), it releases its
/// counts.
struct AdmissionGuard {
    inner: Arc<Inner>,
    slot: Slot,
    stage: Stage,
}

impl AdmissionGuard {
    fn admission(&self) -> MutexGuard<'_, Admission> {
        lock(&self.inner.gateway.admission)
    }

    /// Admits a member upgrade, or `None` when over a cap.
    fn admit_member(inner: &Arc<Inner>, workspace_id: &str) -> Option<AdmissionGuard> {
        let cfg = &inner.cfg;
        let mut admission = lock(&inner.gateway.admission);
        let key = workspace_id.to_owned();
        let overloaded = admission.active + admission.pending >= cfg.max_connections
            || count(&admission.member_by_workspace, &key)
                + count(&admission.member_pending_by_workspace, &key)
                >= cfg.max_connections_per_workspace;
        if overloaded {
            return None;
        }
        admission.pending += 1;
        increment(&mut admission.member_pending_by_workspace, &key);
        Some(AdmissionGuard {
            inner: Arc::clone(inner),
            slot: Slot::Member { workspace_id: key },
            stage: Stage::Pending,
        })
    }

    /// Admits a guest upgrade, or `None` when over a cap.
    fn admit_guest(
        inner: &Arc<Inner>,
        workspace_id: &str,
        guest_conversation_id: &str,
    ) -> Option<AdmissionGuard> {
        let cfg = &inner.cfg;
        let mut admission = lock(&inner.gateway.admission);
        let workspace = workspace_id.to_owned();
        let key = (workspace.clone(), guest_conversation_id.to_owned());
        let overloaded = admission.active + admission.pending >= cfg.max_connections
            || admission.active_guests + admission.pending_guests >= cfg.max_guest_connections
            || count(&admission.guest_by_key, &key) + count(&admission.guest_pending_by_key, &key)
                >= cfg.max_connections_per_guest
            || count(&admission.guest_by_workspace, &workspace)
                + count(&admission.guest_pending_by_workspace, &workspace)
                >= cfg.max_guest_connections_per_workspace;
        if overloaded {
            return None;
        }
        admission.pending += 1;
        admission.pending_guests += 1;
        increment(&mut admission.guest_pending_by_key, &key);
        increment(&mut admission.guest_pending_by_workspace, &workspace);
        Some(AdmissionGuard {
            inner: Arc::clone(inner),
            slot: Slot::Guest {
                workspace_id: workspace,
                guest_conversation_id: key.1,
            },
            stage: Stage::Pending,
        })
    }

    /// An overloaded upgrade (pending until it opens and closes 4003).
    fn overloaded(inner: &Arc<Inner>, guest: bool) -> AdmissionGuard {
        let mut admission = lock(&inner.gateway.admission);
        admission.pending += 1;
        if guest {
            admission.pending_guests += 1;
        }
        AdmissionGuard {
            inner: Arc::clone(inner),
            slot: Slot::Overloaded { guest },
            stage: Stage::Pending,
        }
    }

    fn release_pending(admission: &mut Admission, slot: &Slot) {
        admission.pending = admission.pending.saturating_sub(1);
        match slot {
            Slot::Member { workspace_id } => {
                decrement(&mut admission.member_pending_by_workspace, workspace_id);
            }
            Slot::Guest {
                workspace_id,
                guest_conversation_id,
            } => {
                admission.pending_guests = admission.pending_guests.saturating_sub(1);
                decrement(
                    &mut admission.guest_pending_by_key,
                    &(workspace_id.clone(), guest_conversation_id.clone()),
                );
                decrement(&mut admission.guest_pending_by_workspace, workspace_id);
            }
            Slot::Overloaded { guest } => {
                if *guest {
                    admission.pending_guests = admission.pending_guests.saturating_sub(1);
                }
            }
        }
    }

    /// `closeIfOverloadedOnOpen` for an admitted socket: pending -> active.
    fn activate(&mut self) {
        if self.stage != Stage::Pending {
            return;
        }
        let mut admission = lock(&self.inner.gateway.admission);
        Self::release_pending(&mut admission, &self.slot);
        admission.active += 1;
        admission.peak = admission.peak.max(admission.active);
        match &self.slot {
            Slot::Member { workspace_id } => {
                increment(&mut admission.member_by_workspace, workspace_id);
            }
            Slot::Guest {
                workspace_id,
                guest_conversation_id,
            } => {
                admission.active_guests += 1;
                increment(
                    &mut admission.guest_by_key,
                    &(workspace_id.clone(), guest_conversation_id.clone()),
                );
                increment(&mut admission.guest_by_workspace, workspace_id);
            }
            Slot::Overloaded { .. } => {}
        }
        drop(admission);
        self.stage = Stage::Active;
    }
}

impl Drop for AdmissionGuard {
    fn drop(&mut self) {
        let mut admission = self.admission();
        match self.stage {
            Stage::Pending => Self::release_pending(&mut admission, &self.slot),
            Stage::Active => {
                admission.active = admission.active.saturating_sub(1);
                match &self.slot {
                    Slot::Member { workspace_id } => {
                        decrement(&mut admission.member_by_workspace, workspace_id);
                    }
                    Slot::Guest {
                        workspace_id,
                        guest_conversation_id,
                    } => {
                        admission.active_guests = admission.active_guests.saturating_sub(1);
                        decrement(
                            &mut admission.guest_by_key,
                            &(workspace_id.clone(), guest_conversation_id.clone()),
                        );
                        decrement(&mut admission.guest_by_workspace, workspace_id);
                    }
                    Slot::Overloaded { .. } => {}
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// The outbound queue of one connection
// ---------------------------------------------------------------------------

enum Outbound {
    Frame(Utf8Bytes, usize),
    Ping,
    Close(u16, String),
    /// The peer left: flush (close reply) and stop.
    Shutdown,
}

/// The production [`SocketSink`]: a queue drained by the writer task, with
/// the slow-consumer byte budget.
struct ConnSink {
    tx: mpsc::UnboundedSender<Outbound>,
    pending_bytes: AtomicUsize,
    close_requested: Notify,
}

impl ConnSink {
    fn new() -> (Arc<ConnSink>, mpsc::UnboundedReceiver<Outbound>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (
            Arc::new(ConnSink {
                tx,
                pending_bytes: AtomicUsize::new(0),
                close_requested: Notify::new(),
            }),
            rx,
        )
    }
}

impl SocketSink for ConnSink {
    fn send(&self, frame: &str) -> Result<SendStatus, BoxError> {
        let len = frame.len();
        let pending = self.pending_bytes.load(Ordering::SeqCst);
        // As uWS: a frame is refused only while bytes are already queued.
        if pending > 0 && pending + len > SLOW_CONSUMER_BUFFER_BYTES {
            return Ok(SendStatus::Dropped);
        }
        self.pending_bytes.fetch_add(len, Ordering::SeqCst);
        if self
            .tx
            .send(Outbound::Frame(Utf8Bytes::from(frame.to_owned()), len))
            .is_err()
        {
            self.pending_bytes.fetch_sub(len, Ordering::SeqCst);
            return Err("the socket writer has stopped".into());
        }
        Ok(SendStatus::Sent)
    }

    fn end(&self, code: u16, reason: &str) {
        let _ = self.tx.send(Outbound::Close(code, reason.to_owned()));
        self.close_requested.notify_one();
    }
}

/// Drains the outbound queue into the socket.
async fn write_loop(
    mut sink: futures_util::stream::SplitSink<WebSocket, Message>,
    mut rx: mpsc::UnboundedReceiver<Outbound>,
    conn: Arc<ConnSink>,
) {
    while let Some(outbound) = rx.recv().await {
        match outbound {
            Outbound::Frame(text, len) => {
                let written =
                    tokio::time::timeout(WRITE_TIMEOUT, sink.send(Message::Text(text))).await;
                conn.pending_bytes.fetch_sub(len, Ordering::SeqCst);
                if !matches!(written, Ok(Ok(()))) {
                    return;
                }
            }
            Outbound::Ping => {
                let written =
                    tokio::time::timeout(WRITE_TIMEOUT, sink.send(Message::Ping(Bytes::new())))
                        .await;
                if !matches!(written, Ok(Ok(()))) {
                    return;
                }
            }
            Outbound::Close(code, reason) => {
                let frame = CloseFrame {
                    code,
                    reason: Utf8Bytes::from(reason),
                };
                let _ = tokio::time::timeout(CLOSE_TIMEOUT, sink.send(Message::Close(Some(frame))))
                    .await;
                let _ = tokio::time::timeout(CLOSE_TIMEOUT, sink.close()).await;
                return;
            }
            Outbound::Shutdown => break,
        }
    }
    let _ = tokio::time::timeout(CLOSE_TIMEOUT, sink.close()).await;
}

// ---------------------------------------------------------------------------
// Connections
// ---------------------------------------------------------------------------

/// What the connection loop needs from a member or guest socket.
trait ConnSocket: Send + Sync + 'static {
    fn is_closed(&self) -> bool;
    fn close(&self, code: u16, reason: &str) -> bool;
    fn mark_closed(&self);
    fn deliver(&self, delivery: &Delivery, entry: &Entry);
}

impl ConnSocket for MemberSocket {
    fn is_closed(&self) -> bool {
        MemberSocket::is_closed(self)
    }
    fn close(&self, code: u16, reason: &str) -> bool {
        MemberSocket::close(self, code, reason)
    }
    fn mark_closed(&self) {
        MemberSocket::mark_closed(self);
    }
    fn deliver(&self, delivery: &Delivery, entry: &Entry) {
        delivery.deliver_member(self, entry);
    }
}

impl ConnSocket for GuestSocket {
    fn is_closed(&self) -> bool {
        GuestSocket::is_closed(self)
    }
    fn close(&self, code: u16, reason: &str) -> bool {
        GuestSocket::close(self, code, reason)
    }
    fn mark_closed(&self) {
        GuestSocket::mark_closed(self);
    }
    fn deliver(&self, delivery: &Delivery, entry: &Entry) {
        delivery.deliver_guest(self, entry);
    }
}

/// Everything an admitted upgrade carries into its socket task.
struct Admitted<S> {
    socket: Arc<S>,
    conn: Arc<ConnSink>,
    outbound: mpsc::UnboundedReceiver<Outbound>,
    attached: Attached,
    admission: AdmissionGuard,
    workspace_ref: WorkspaceRef,
}

/// The socket task: open steps, then the live loop until a close.
async fn run_connection<S: ConnSocket>(
    rt: Realtime,
    websocket: WebSocket,
    admitted: Admitted<S>,
    open: impl FnOnce(&Realtime, &Arc<S>, &Attached) -> OpenOutcome + Send + 'static,
    on_last_member_socket: impl FnOnce(&Realtime) + Send + 'static,
) {
    let Admitted {
        socket,
        conn,
        outbound,
        mut attached,
        mut admission,
        workspace_ref,
    } = admitted;
    let gateway = &rt.inner.gateway;
    admission.activate();
    rt.counters().upgrades.fetch_add(1, Ordering::Relaxed);
    gateway.socket_opened();

    let outcome = open(&rt, &socket, &attached);
    let (sink, mut reader) = websocket.split();
    let mut writer = tokio::spawn(write_loop(sink, outbound, Arc::clone(&conn)));
    let mut writer_done = false;
    let mut peer_gone = false;

    if !socket.is_closed() {
        let lifetime = async {
            match outcome {
                OpenOutcome::Open { lifetime_ms } => {
                    tokio::time::sleep(Duration::from_millis(lifetime_ms)).await;
                }
                OpenOutcome::Closed => std::future::pending::<()>().await,
            }
        };
        tokio::pin!(lifetime);
        let mut heartbeat = gateway.heartbeat_tx.subscribe();
        heartbeat.borrow_and_update();
        let idle_timeout = Duration::from_millis(rt.inner.cfg.idle_timeout_ms);
        let mut last_inbound = Instant::now();
        let delivery = rt.hub().delivery();
        loop {
            tokio::select! {
                biased;
                () = conn.close_requested.notified() => {}
                message = reader.next() => match message {
                    Some(Ok(Message::Close(_))) | Some(Err(_)) | None => {
                        peer_gone = true;
                        socket.mark_closed();
                    }
                    // The socket is server -> client: client frames only
                    // prove the peer is alive.
                    Some(Ok(_)) => last_inbound = Instant::now(),
                },
                received = attached.rx.recv() => match received {
                    Ok(entry) => socket.deliver(delivery, &entry),
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        rt.counters().drops.fetch_add(1, Ordering::Relaxed);
                        socket.close(close::RESYNC, reason::BACKPRESSURE_DROP);
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        socket.close(close::RESYNC, reason::SEND_FAILED);
                    }
                },
                changed = heartbeat.changed() => {
                    if changed.is_err() || last_inbound.elapsed() >= idle_timeout {
                        // uWS `idleTimeout`: a silent peer is dropped.
                        peer_gone = true;
                        socket.mark_closed();
                    } else {
                        match conn.send(HEARTBEAT_FRAME) {
                            Ok(SendStatus::Sent) => {
                                let _ = conn.tx.send(Outbound::Ping);
                            }
                            Ok(SendStatus::Dropped) => {
                                rt.counters().drops.fetch_add(1, Ordering::Relaxed);
                                socket.close(close::RESYNC, reason::BACKPRESSURE_DROP);
                            }
                            Err(_) => {
                                socket.close(close::RESYNC, reason::SEND_FAILED);
                            }
                        }
                    }
                },
                () = &mut lifetime => {
                    socket.close(close::REAUTH, reason::CONNECTION_LIFETIME_EXCEEDED);
                },
                _ = &mut writer => {
                    writer_done = true;
                    peer_gone = true;
                    socket.mark_closed();
                },
            }
            if socket.is_closed() {
                break;
            }
        }
    }

    // Release before the TCP close, so a reconnect finds the slot free.
    if attached.registration.unregister() {
        on_last_member_socket(&rt);
    }
    gateway.socket_closed();
    drop(admission);
    drop(workspace_ref);
    drop(attached);

    if peer_gone {
        let _ = conn.tx.send(Outbound::Shutdown);
    } else {
        // Server-initiated close: wait briefly for the peer's close reply.
        let _ = tokio::time::timeout(CLOSE_TIMEOUT, drain_reader(&mut reader)).await;
    }
    if !writer_done
        && tokio::time::timeout(CLOSE_TIMEOUT, &mut writer)
            .await
            .is_err()
    {
        writer.abort();
    }
}

async fn drain_reader(reader: &mut SplitStream<WebSocket>) {
    while let Some(Ok(message)) = reader.next().await {
        if matches!(message, Message::Close(_)) {
            return;
        }
    }
}

/// Upgrades, then closes 4003 `{"retryAfter":..}` at once.
fn overloaded_upgrade(rt: &Realtime, upgrade: WebSocketUpgrade, guest: bool) -> Response {
    let guard = AdmissionGuard::overloaded(&rt.inner, guest);
    let counters = Arc::clone(rt.counters());
    configure(upgrade).on_upgrade(move |mut websocket| async move {
        drop(guard);
        counters.overload_closes.fetch_add(1, Ordering::Relaxed);
        let frame = CloseFrame {
            code: close::OVERLOADED,
            reason: Utf8Bytes::from(retry_after_reason(next_overload_retry_after_ms())),
        };
        if tokio::time::timeout(CLOSE_TIMEOUT, websocket.send(Message::Close(Some(frame))))
            .await
            .is_ok()
        {
            let _ = tokio::time::timeout(CLOSE_TIMEOUT, async {
                while let Some(Ok(message)) = websocket.recv().await {
                    if matches!(message, Message::Close(_)) {
                        break;
                    }
                }
            })
            .await;
        }
    })
}

fn configure(upgrade: WebSocketUpgrade) -> WebSocketUpgrade {
    upgrade
        .read_buffer_size(4096)
        .write_buffer_size(0)
        .max_message_size(MAX_CLIENT_MESSAGE_BYTES)
        .max_frame_size(MAX_CLIENT_MESSAGE_BYTES)
}

// ---------------------------------------------------------------------------
// Requests
// ---------------------------------------------------------------------------

fn status(code: StatusCode) -> Response {
    code.into_response()
}

/// The raw last path segment (the route parameter; never holds a `/`).
fn last_segment(uri: &Uri) -> &str {
    uri.path().rsplit('/').next().unwrap_or("")
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// Percent-decoding; `None` on a bad escape or bad UTF-8
/// (`decodeURIComponent` throws). `plus_as_space` for query values.
fn percent_decode(raw: &str, plus_as_space: bool) -> Option<String> {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' => {
                let high = hex_value(*bytes.get(index + 1)?)?;
                let low = hex_value(*bytes.get(index + 2)?)?;
                out.push(high * 16 + low);
                index += 3;
            }
            b'+' if plus_as_space => {
                out.push(b' ');
                index += 1;
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

/// `req.getQuery(name)`: the first decoded value.
fn query_value(uri: &Uri, name: &str) -> Option<String> {
    uri.query()?.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let key = percent_decode(key, true).unwrap_or_else(|| key.to_owned());
        (key == name).then(|| percent_decode(value, true).unwrap_or_else(|| value.to_owned()))
    })
}

enum RevokeCheck {
    Ok,
    Revoked,
    StoreError,
}

/// `checkMemberRevoked`: `revokedAt > iatMs` (strict); a non-numeric
/// marker counts as revoked; a read error fails closed.
async fn check_member_revoked(
    rt: &Realtime,
    workspace_id: &str,
    claims: &MemberClaims,
) -> RevokeCheck {
    match rt
        .inner
        .deps
        .revoke_store
        .read_marker(workspace_id, &claims.user_id)
        .await
    {
        Err(err) => {
            tracing::warn!(
                err = %err,
                workspace_id = %workspace_id,
                "Realtime revoke-marker read failed"
            );
            RevokeCheck::StoreError
        }
        Ok(None) => RevokeCheck::Ok,
        Ok(Some(raw)) if raw.is_empty() => RevokeCheck::Ok,
        Ok(Some(raw)) => match raw.trim().parse::<f64>() {
            Ok(revoked_at) if !revoked_at.is_nan() => {
                if revoked_at > claims.iat_ms as f64 {
                    RevokeCheck::Revoked
                } else {
                    RevokeCheck::Ok
                }
            }
            _ => {
                tracing::warn!(
                    revoked_at_raw = %raw,
                    user_id = %claims.user_id,
                    workspace_id = %workspace_id,
                    "Realtime revoke marker is not a number"
                );
                RevokeCheck::Revoked
            }
        },
    }
}

fn revoke_refusal(rt: &Realtime, check: &RevokeCheck) -> Option<Response> {
    match check {
        RevokeCheck::Ok => None,
        RevokeCheck::Revoked => {
            rt.counters()
                .token_rejections
                .fetch_add(1, Ordering::Relaxed);
            Some(status(StatusCode::UNAUTHORIZED))
        }
        RevokeCheck::StoreError => Some(status(StatusCode::SERVICE_UNAVAILABLE)),
    }
}

fn reject_token(rt: &Realtime, route: Route<'_>, err: &dyn std::fmt::Display) -> Response {
    rt.counters()
        .token_rejections
        .fetch_add(1, Ordering::Relaxed);
    rt.inner
        .gateway
        .log_token_rejection(rt.clock().now_ms(), route, err);
    status(StatusCode::UNAUTHORIZED)
}

/// `GET /rt/workspaces/{workspace_id}`.
async fn member_upgrade(
    State(rt): State<Realtime>,
    uri: Uri,
    upgrade: WebSocketUpgrade,
) -> Response {
    let workspace_id = last_segment(&uri).to_owned();
    let token = query_value(&uri, "token").filter(|token| !token.is_empty());
    let last_seq = query_value(&uri, "lastSeq");
    let Some(token) = token.filter(|_| !workspace_id.is_empty()) else {
        rt.counters()
            .token_rejections
            .fetch_add(1, Ordering::Relaxed);
        return status(StatusCode::UNAUTHORIZED);
    };
    let claims = match verify_member_connect_token(
        &token,
        &workspace_id,
        &rt.inner.cfg.secret,
        &**rt.clock(),
    ) {
        Ok(claims) => claims,
        Err(err) => return reject_token(&rt, Route::Workspace(&workspace_id), &err),
    };
    if rt.inner.gateway.is_stopping() {
        return overloaded_upgrade(&rt, upgrade, false);
    }
    let Some(admission) = AdmissionGuard::admit_member(&rt.inner, &workspace_id) else {
        return overloaded_upgrade(&rt, upgrade, false);
    };
    let workspace_ref = rt.hub().retain(&workspace_id);

    let check = check_member_revoked(&rt, &workspace_id, &claims).await;
    if let Some(refusal) = revoke_refusal(&rt, &check) {
        return refusal;
    }
    let (conn, outbound) = ConnSink::new();
    let socket = MemberSocket::new(
        MemberSocketData {
            workspace_id: workspace_id.clone(),
            user_id: claims.user_id.clone(),
            chat_scope: claims.chat_scope,
            team_ids: claims.team_ids.clone(),
            iat_ms: claims.iat_ms,
        },
        conn.clone(),
    );
    // Registered before the recheck: a revoke landing after it closes the
    // registered socket (spec 5.2 item 7).
    let attached = rt.attach_member_socket(&socket, last_seq.as_deref());
    let recheck = check_member_revoked(&rt, &workspace_id, &claims).await;
    if let Some(refusal) = revoke_refusal(&rt, &recheck) {
        return refusal;
    }
    let admitted = Admitted {
        socket,
        conn,
        outbound,
        attached,
        admission,
        workspace_ref,
    };
    let user_id = claims.user_id;
    configure(upgrade).on_upgrade(move |websocket| {
        run_connection(
            rt,
            websocket,
            admitted,
            |rt, socket, attached| rt.open_member_socket(socket, attached),
            move |rt| rt.inner.gateway.presence_remove(&workspace_id, &user_id),
        )
    })
}

/// `GET /rt/guests/{guest_conversation_id}` (the id is percent-decoded).
async fn guest_upgrade(
    State(rt): State<Realtime>,
    uri: Uri,
    upgrade: WebSocketUpgrade,
) -> Response {
    let guest_conversation_id =
        percent_decode(last_segment(&uri), false).filter(|id| !id.is_empty());
    let token = query_value(&uri, "token").filter(|token| !token.is_empty());
    let last_seq = query_value(&uri, "lastSeq");
    let (Some(guest_conversation_id), Some(token)) = (guest_conversation_id, token) else {
        rt.counters()
            .token_rejections
            .fetch_add(1, Ordering::Relaxed);
        return status(StatusCode::UNAUTHORIZED);
    };
    let claims = match verify_guest_connect_token(
        &token,
        &guest_conversation_id,
        &rt.inner.cfg.secret,
        &**rt.clock(),
    ) {
        Ok(claims) => claims,
        Err(err) => return reject_token(&rt, Route::Guest(&guest_conversation_id), &err),
    };
    if rt.inner.gateway.is_stopping() {
        return overloaded_upgrade(&rt, upgrade, true);
    }
    let Some(admission) =
        AdmissionGuard::admit_guest(&rt.inner, &claims.workspace_id, &guest_conversation_id)
    else {
        return overloaded_upgrade(&rt, upgrade, true);
    };
    let workspace_ref = rt.hub().retain(&claims.workspace_id);
    let (conn, outbound) = ConnSink::new();
    let socket = GuestSocket::new(
        GuestSocketData {
            workspace_id: claims.workspace_id,
            guest_conversation_id,
        },
        conn.clone(),
    );
    let attached = rt.hub().attach_guest(&socket, last_seq.as_deref());
    let admitted = Admitted {
        socket,
        conn,
        outbound,
        attached,
        admission,
        workspace_ref,
    };
    configure(upgrade).on_upgrade(move |websocket| {
        run_connection(
            rt,
            websocket,
            admitted,
            |rt, socket, attached| rt.open_guest_socket(socket, attached),
            |_| {},
        )
    })
}

/// `GET /health`: always `200 ok`.
async fn health() -> &'static str {
    "ok"
}

/// `GET /ready`: `200 ready` while serving, `503 not ready` once shutdown
/// started.
async fn ready(State(rt): State<Realtime>) -> Response {
    if rt.is_ready() {
        "ready".into_response()
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, "not ready").into_response()
    }
}

impl Realtime {
    /// `connectionLifetimeMs ± 15 %`.
    fn jittered_lifetime_ms(&self) -> u64 {
        let lifetime = self.inner.cfg.connection_lifetime_ms as f64;
        let jitter =
            ((random_unit() * 2.0 - 1.0) * lifetime * CONNECTION_LIFETIME_JITTER_RATIO).floor();
        (lifetime + jitter).max(1.0) as u64
    }

    /// Snapshot + subscribe + register of a member socket (spec 3.4 step 5).
    pub fn attach_member_socket(
        &self,
        socket: &Arc<MemberSocket>,
        last_seq: Option<&str>,
    ) -> Attached {
        self.hub().attach_member(socket, last_seq)
    }

    /// The open-time steps of a member socket (spec 3.4 step 7): close
    /// reason, replay, cursor frame, presence for the user's first socket,
    /// lifetime. A send failure closes 4002 and skips the later steps.
    pub fn open_member_socket(
        &self,
        socket: &Arc<MemberSocket>,
        attached: &Attached,
    ) -> OpenOutcome {
        let delivery = self.hub().delivery();
        if let Some(reason) = attached.replay.close_reason {
            socket.close(close::RESYNC, reason);
            return OpenOutcome::Closed;
        }
        if !delivery.replay_workspace_socket(socket, &attached.replay.entries) {
            return OpenOutcome::Closed;
        }
        delivery.send_cursor(socket, &attached.replay.cursor.to_string());
        if socket.is_closed() {
            return OpenOutcome::Closed;
        }
        let data = socket.data();
        self.inner
            .gateway
            .presence_add(&data.workspace_id, &data.user_id);
        OpenOutcome::Open {
            lifetime_ms: self.jittered_lifetime_ms(),
        }
    }

    /// The open-time steps of a guest socket: close reason, replay,
    /// lifetime (no cursor frame, no presence).
    fn open_guest_socket(&self, socket: &Arc<GuestSocket>, attached: &Attached) -> OpenOutcome {
        if let Some(reason) = attached.replay.close_reason {
            socket.close(close::RESYNC, reason);
            return OpenOutcome::Closed;
        }
        if !self
            .hub()
            .delivery()
            .replay_guest_socket(socket, &attached.replay.entries)
        {
            return OpenOutcome::Closed;
        }
        OpenOutcome::Open {
            lifetime_ms: self.jittered_lifetime_ms(),
        }
    }

    /// Member and guest upgrades admitted but not open yet.
    pub fn pending_upgrades(&self) -> usize {
        lock(&self.inner.gateway.admission).pending
    }

    /// Guest upgrades admitted but not open yet.
    pub fn pending_guest_upgrades(&self) -> usize {
        lock(&self.inner.gateway.admission).pending_guests
    }

    /// Open sockets (members + guests).
    pub fn active_connections(&self) -> usize {
        lock(&self.inner.gateway.admission).active
    }

    /// The users presence currently reports for `workspace_id`.
    pub fn presence_users(&self, workspace_id: &str) -> Vec<String> {
        let mut users: Vec<String> = lock(&self.inner.gateway.presence.users)
            .get(workspace_id)
            .map(|users| users.iter().cloned().collect())
            .unwrap_or_default();
        users.sort();
        users
    }

    /// Presence coalesce timers waiting to fire.
    pub fn pending_presence_reports(&self) -> usize {
        lock(&self.inner.gateway.presence.timers).len()
    }

    /// Fires every pending presence coalesce timer now.
    pub fn flush_presence_reports(&self) {
        let workspace_ids: Vec<String> = {
            let mut timers = lock(&self.inner.gateway.presence.timers);
            timers
                .drain()
                .map(|(workspace_id, (_, timer))| {
                    timer.abort();
                    workspace_id
                })
                .collect()
        };
        for workspace_id in workspace_ids {
            self.inner
                .gateway
                .presence
                .reporter
                .report_now(&workspace_id);
        }
    }

    /// Whether the shared heartbeat task is running (only with open sockets).
    pub fn heartbeat_running(&self) -> bool {
        lock(&self.inner.gateway.heartbeat).task.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RtConfig;
    use crate::presence::NoPresence;
    use crate::protocol::{Envelope, EventType};
    use crate::revoke::{NoRevokeRetry, RevokeStore};
    use crate::token::{ChatScope, MemberConnect, sign_member_connect_token};
    use chatbotx_kv::{BoxFuture, Clock, SystemClock};
    use serde_json::json;
    use tokio_tungstenite::tungstenite::Message as ClientMessage;

    struct NoMarkers;

    impl RevokeStore for NoMarkers {
        fn read_marker<'a>(
            &'a self,
            _workspace_id: &'a str,
            _user_id: &'a str,
        ) -> BoxFuture<'a, Result<Option<String>, BoxError>> {
            Box::pin(async { Ok(None) })
        }

        fn raise_marker<'a>(
            &'a self,
            _workspace_id: &'a str,
            _user_id: &'a str,
            _revoked_at_ms: u64,
        ) -> BoxFuture<'a, Result<(), BoxError>> {
            Box::pin(async { Ok(()) })
        }
    }

    const SECRET: &str = "unit-test-secret-unit-test-secret";

    async fn serve(cfg: RtConfig) -> (Realtime, u16) {
        let rt = Realtime::with_deps(
            cfg,
            RealtimeDeps {
                clock: Arc::new(SystemClock),
                revoke_store: Arc::new(NoMarkers),
                presence: Arc::new(NoPresence),
                revoke_retry: Arc::new(NoRevokeRetry),
            },
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let app = router(rt.clone());
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (rt, port)
    }

    fn config() -> RtConfig {
        let mut cfg = RtConfig::new(SECRET);
        cfg.shutdown_drain_ms = 0;
        cfg
    }

    fn member_url(port: u16, scope: ChatScope, user_id: &str) -> String {
        let token = sign_member_connect_token(
            &MemberConnect {
                workspace_id: "ws-1".to_owned(),
                user_id: user_id.to_owned(),
                chat_scope: scope,
                team_ids: Vec::new(),
                issued_at_ms: SystemClock.now_ms(),
            },
            SECRET,
        )
        .unwrap();
        format!("ws://127.0.0.1:{port}/rt/workspaces/ws-1?token={token}")
    }

    type Client = tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >;

    async fn connect(url: &str) -> Client {
        tokio_tungstenite::connect_async(url).await.unwrap().0
    }

    /// The next text frame, or the close `(code, reason)` as `Err`.
    async fn next(client: &mut Client) -> Result<String, (u16, String)> {
        let read = async {
            loop {
                match client.next().await {
                    Some(Ok(ClientMessage::Text(text))) => return Ok(text.to_string()),
                    Some(Ok(ClientMessage::Close(frame))) => {
                        return Err(frame.map_or((1005, String::new()), |frame| {
                            (u16::from(frame.code), frame.reason.to_string())
                        }));
                    }
                    Some(Ok(_)) => continue,
                    Some(Err(_)) | None => return Err((1006, String::new())),
                }
            }
        };
        tokio::time::timeout(Duration::from_secs(10), read)
            .await
            .expect("no frame in time")
    }

    async fn wait_until(mut condition: impl FnMut() -> bool) {
        for _ in 0..1000 {
            if condition() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("condition not met");
    }

    fn typing() -> Envelope {
        serde_json::from_value(json!({ "data": { "typing": true }, "eventType": "typing" }))
            .unwrap()
    }

    #[test]
    fn the_outbound_queue_drops_frames_over_the_slow_consumer_budget() {
        let (conn, _rx) = ConnSink::new();
        let big = "x".repeat(600_000);
        // A single frame passes while nothing is queued (as uWS).
        assert_eq!(conn.send(&big).unwrap(), SendStatus::Sent);
        assert_eq!(conn.send("y").unwrap(), SendStatus::Dropped);
        let (conn, _rx) = ConnSink::new();
        let half = "x".repeat(300_000);
        assert_eq!(conn.send(&half).unwrap(), SendStatus::Sent);
        assert_eq!(conn.send(&half).unwrap(), SendStatus::Dropped);
        drop(_rx);
        let (conn, rx) = ConnSink::new();
        drop(rx);
        assert!(conn.send("z").is_err());
    }

    #[test]
    fn percent_decoding_follows_decode_uri_component() {
        assert_eq!(
            percent_decode("ws-1%3Aguest-1", false).as_deref(),
            Some("ws-1:guest-1")
        );
        assert_eq!(percent_decode("a+b", false).as_deref(), Some("a+b"));
        assert_eq!(percent_decode("a+b", true).as_deref(), Some("a b"));
        assert_eq!(percent_decode("bad%zz", false), None);
        assert_eq!(percent_decode("bad%", false), None);
        assert_eq!(percent_decode("%C3%28", false), None);
    }

    #[test]
    fn jittered_values_stay_in_range() {
        for _ in 0..1000 {
            let retry_after = next_overload_retry_after_ms();
            assert!((1000..5000).contains(&retry_after));
        }
    }

    #[tokio::test]
    async fn idle_runs_no_timer_and_the_heartbeat_and_sweep_follow_open_sockets() {
        let (rt, port) = serve(config()).await;
        assert!(!rt.heartbeat_running());
        assert!(lock(&rt.inner.gateway.presence.sweep).is_none());
        assert!(!rt.hub().maintenance_running());

        let client = connect(&member_url(port, ChatScope::All, "user-1")).await;
        wait_until(|| rt.heartbeat_running() && rt.active_connections() == 1).await;
        assert!(lock(&rt.inner.gateway.presence.sweep).is_some());

        drop(client);
        wait_until(|| !rt.heartbeat_running() && rt.active_connections() == 0).await;
        assert!(lock(&rt.inner.gateway.presence.sweep).is_none());
        assert_eq!(rt.hub().bus_count(), 0);
        rt.shutdown().await;
    }

    #[tokio::test]
    async fn live_records_reach_open_sockets_with_the_scope_rules() {
        let (rt, port) = serve(config()).await;
        let mut all = connect(&member_url(port, ChatScope::All, "user-1")).await;
        let mut none = connect(&member_url(port, ChatScope::None, "user-2")).await;
        assert_eq!(next(&mut all).await.unwrap(), r#"{"batch":[],"seq":"0-0"}"#);
        assert_eq!(
            next(&mut none).await.unwrap(),
            r#"{"batch":[],"seq":"0-0"}"#
        );
        wait_until(|| rt.active_connections() == 2).await;

        rt.publish_workspace_event("ws-1", typing()).await.unwrap();
        let frame = next(&mut all).await.unwrap();
        assert!(
            frame
                .starts_with(r#"{"batch":[{"data":{"typing":true},"eventType":"typing"}],"seq":""#)
        );

        // `chatScope: none` gets no workspace events, only its own sends.
        rt.publish_member_event(
            "ws-1",
            "user-2",
            Envelope::new(
                EventType::WhatsappCallOutboundStatus,
                json!({ "attemptId": "a" }),
            ),
        )
        .unwrap();
        let direct = next(&mut none).await.unwrap();
        assert!(direct.contains("whatsappCallOutboundStatus"), "{direct}");

        // A live revoke closes the member's socket 4001.
        rt.revoke_member(crate::revoke::RevokeArgs {
            workspace_id: "ws-1".to_owned(),
            user_id: "user-1".to_owned(),
            reason: crate::revoke::RevokeReason::Deleted,
            revoked_at_ms: None,
        })
        .await
        .unwrap();
        assert_eq!(next(&mut all).await, Err((4001, "revoked".to_owned())));
        rt.shutdown().await;
        assert_eq!(next(&mut none).await.map_err(|(code, _)| code), Err(4003));
    }

    #[tokio::test]
    async fn a_lagging_socket_is_closed_with_backpressure_drop() {
        let (rt, port) = serve(config()).await;
        let mut client = connect(&member_url(port, ChatScope::All, "user-1")).await;
        assert!(next(&mut client).await.is_ok());
        wait_until(|| rt.active_connections() == 1).await;

        // No yield between appends: the connection task falls 300 records
        // behind a 256-slot channel.
        for _ in 0..300 {
            rt.hub().append_workspace_events("ws-1", vec![typing()]);
        }

        loop {
            match next(&mut client).await {
                Ok(_) => continue,
                Err(close) => {
                    assert_eq!(close, (4002, "backpressure-drop".to_owned()));
                    break;
                }
            }
        }
        rt.shutdown().await;
    }

    #[tokio::test]
    async fn heartbeats_go_out_and_a_silent_peer_is_dropped() {
        let mut cfg = config();
        cfg.heartbeat_interval_ms = 40;
        cfg.idle_timeout_ms = 150;
        let (rt, port) = serve(cfg).await;
        let mut client = connect(&member_url(port, ChatScope::All, "user-1")).await;
        assert!(next(&mut client).await.is_ok());
        assert_eq!(next(&mut client).await.unwrap(), HEARTBEAT_FRAME);

        // Stop reading (no pong): the server drops the socket after the
        // idle timeout without a close frame.
        tokio::time::sleep(Duration::from_millis(400)).await;
        wait_until(|| rt.active_connections() == 0).await;
        let mut ended = false;
        for _ in 0..50 {
            if let Err((code, _)) = next(&mut client).await {
                assert_eq!(code, 1006);
                ended = true;
                break;
            }
        }
        assert!(ended);
        rt.shutdown().await;
    }
}

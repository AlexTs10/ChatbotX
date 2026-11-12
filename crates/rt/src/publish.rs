//! The producer API (spec 1.6, 3.5; `realtime-broadcast.ts`): the
//! [`Realtime`] handle that `core` and `jobs` call in the same process.
//!
//! Workspace events are coalesced per workspace for 25 ms, at most 64
//! events or 256 KiB per record; the immediate types flush at once. The
//! append is a synchronous call under the hub lock, so it cannot fail or
//! stay in flight; validation still rejects one event without delaying the
//! others.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::Duration;

use chatbotx_kv::{BoxFuture, Clock, Kv};
use serde_json::Value;
use tokio::sync::watch;
use tokio::task::AbortHandle;

use crate::config::RtConfig;
use crate::delivery::{Counters, Delivery};
use crate::gateway::{GatewayState, next_overload_retry_after_ms};
use crate::hub::{Hub, HubConfig};
use crate::presence::PresenceSink;
use crate::protocol::{
    ConversationRoute, Envelope, EventType, ProtocolError, flushes_immediately,
    route_for_conversation, to_json, validate_guest_envelope, validate_workspace_envelope,
};
use crate::relay_metrics::RelayMetrics;
use crate::revoke::{
    REVOKE_RETRY_ATTEMPTS, REVOKE_RETRY_DELAY_MS, RevokeArgs, RevokeError, RevokeRetry, RevokeStore,
};

/// `WORKSPACE_REALTIME_COALESCE_MS`.
pub const WORKSPACE_REALTIME_COALESCE_MS: u64 = 25;
/// `WORKSPACE_REALTIME_MAX_EVENTS`.
pub const WORKSPACE_REALTIME_MAX_EVENTS: usize = 64;
/// `WORKSPACE_REALTIME_MAX_BYTES` (of the exact record JSON).
pub const WORKSPACE_REALTIME_MAX_BYTES: usize = 256 * 1024;

/// A rejected publish.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PublishError {
    /// `Invalid workspace realtime event`.
    InvalidWorkspaceEvent(ProtocolError),
    /// `Invalid guest realtime event`.
    InvalidGuestEvent(ProtocolError),
}

impl fmt::Display for PublishError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PublishError::InvalidWorkspaceEvent(err) => {
                write!(f, "Invalid workspace realtime event: {err}")
            }
            PublishError::InvalidGuestEvent(err) => {
                write!(f, "Invalid guest realtime event: {err}")
            }
        }
    }
}

impl std::error::Error for PublishError {}

/// The collaborators of a [`Realtime`] (tests pass fakes).
#[derive(Clone)]
pub struct RealtimeDeps {
    pub clock: Arc<dyn Clock>,
    /// Revoke marker store (production: `kv.durable`).
    pub revoke_store: Arc<dyn RevokeStore>,
    /// Presence report target (production: core's presence service).
    pub presence: Arc<dyn PresenceSink>,
    /// Durable revoke retry (production: the jobs enqueuer).
    pub revoke_retry: Arc<dyn RevokeRetry>,
}

/// The realtime handle: hub, coalescer, presence, revoke, gateway state.
/// `Clone` is one `Arc`.
#[derive(Clone)]
pub struct Realtime {
    pub(crate) inner: Arc<Inner>,
}

pub(crate) struct Inner {
    pub(crate) cfg: RtConfig,
    pub(crate) deps: RealtimeDeps,
    pub(crate) hub: Arc<Hub>,
    pub(crate) relay: RelayMetrics,
    pub(crate) gateway: GatewayState,
    coalescer: Mutex<Coalescer>,
    this: Weak<Inner>,
}

/// One workspace's coalesced batch.
struct PendingBatch {
    id: u64,
    events: Vec<Envelope>,
    events_json: Vec<String>,
    /// Bytes of the exact record JSON so far.
    bytes: usize,
    done: watch::Sender<bool>,
    timer: Option<AbortHandle>,
}

#[derive(Default)]
struct Coalescer {
    next_id: u64,
    batches: HashMap<String, PendingBatch>,
}

/// Bytes of `{"events":[],"kind":"workspace-events","workspaceId":<ws>}`.
fn empty_record_bytes(workspace_id: &str) -> usize {
    r#"{"events":[],"kind":"workspace-events","workspaceId":}"#.len() + to_json(workspace_id).len()
}

impl Inner {
    fn coalescer(&self) -> MutexGuard<'_, Coalescer> {
        self.coalescer
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn now_ms(&self) -> u64 {
        u64::try_from(self.deps.clock.now_ms()).unwrap_or(0)
    }

    /// Appends one batch (taken out of the coalescer) and resolves it.
    fn append_batch(&self, workspace_id: &str, batch: PendingBatch) {
        if let Some(timer) = &batch.timer {
            timer.abort();
        }
        if !batch.events.is_empty() {
            self.relay.record_flush(batch.events.len());
            self.hub.append_workspace_events_serialized(
                workspace_id,
                batch.events,
                batch.events_json.join(","),
            );
        }
        batch.done.send_replace(true);
    }

    /// Flushes the workspace batch now (under the coalescer lock, so the
    /// appends of one workspace keep their order).
    fn flush_locked(&self, coalescer: &mut Coalescer, workspace_id: &str) {
        if let Some(batch) = coalescer.batches.remove(workspace_id) {
            self.append_batch(workspace_id, batch);
        }
    }

    /// The 25 ms timer: flushes batch `id` if it is still pending.
    fn flush_if_current(&self, workspace_id: &str, id: u64) {
        let mut coalescer = self.coalescer();
        if coalescer
            .batches
            .get(workspace_id)
            .is_some_and(|batch| batch.id == id)
        {
            if let Some(batch) = coalescer.batches.get_mut(workspace_id) {
                batch.timer = None;
            }
            self.flush_locked(&mut coalescer, workspace_id);
        }
    }

    fn new_batch(&self, coalescer: &mut Coalescer, workspace_id: &str) {
        coalescer.next_id += 1;
        let id = coalescer.next_id;
        let timer = tokio::runtime::Handle::try_current().ok().map(|handle| {
            let weak = self.this.clone();
            let workspace_id = workspace_id.to_owned();
            handle
                .spawn(async move {
                    tokio::time::sleep(Duration::from_millis(WORKSPACE_REALTIME_COALESCE_MS)).await;
                    if let Some(inner) = weak.upgrade() {
                        inner.flush_if_current(&workspace_id, id);
                    }
                })
                .abort_handle()
        });
        coalescer.batches.insert(
            workspace_id.to_owned(),
            PendingBatch {
                id,
                events: Vec::new(),
                events_json: Vec::new(),
                bytes: empty_record_bytes(workspace_id),
                done: watch::channel(false).0,
                timer,
            },
        );
    }

    /// Adds one validated event to its workspace batch; returns the
    /// delivery receiver of that batch.
    fn enqueue(
        &self,
        workspace_id: &str,
        event: Envelope,
        event_json: String,
    ) -> watch::Receiver<bool> {
        let immediate = flushes_immediately(event.event_type);
        let mut coalescer = self.coalescer();
        let split = coalescer.batches.get(workspace_id).is_some_and(|batch| {
            let separator = usize::from(!batch.events.is_empty());
            batch.bytes + separator + event_json.len() > WORKSPACE_REALTIME_MAX_BYTES
                || batch.events.len() + 1 > WORKSPACE_REALTIME_MAX_EVENTS
        });
        if split {
            self.flush_locked(&mut coalescer, workspace_id);
        }
        if !coalescer.batches.contains_key(workspace_id) {
            self.new_batch(&mut coalescer, workspace_id);
        }
        let Some(batch) = coalescer.batches.get_mut(workspace_id) else {
            unreachable!("created above");
        };
        let separator = usize::from(!batch.events.is_empty());
        batch.bytes += separator + event_json.len();
        batch.events.push(event);
        batch.events_json.push(event_json);
        let done = batch.done.subscribe();
        let full = batch.events.len() == WORKSPACE_REALTIME_MAX_EVENTS;
        // No runtime for the timer: append at once.
        let untimed = batch.timer.is_none();
        if immediate || full || untimed {
            self.flush_locked(&mut coalescer, workspace_id);
        }
        done
    }
}

impl Realtime {
    /// Production start: the revoke marker lives in `kv.durable`. Must run
    /// inside a tokio runtime.
    pub fn start(
        kv: Arc<Kv>,
        clock: Arc<dyn Clock>,
        cfg: RtConfig,
        presence: Arc<dyn PresenceSink>,
        revoke_retry: Arc<dyn RevokeRetry>,
    ) -> Realtime {
        Realtime::with_deps(
            cfg,
            RealtimeDeps {
                clock,
                revoke_store: Arc::new(kv.durable.clone()),
                presence,
                revoke_retry,
            },
        )
    }

    /// Start with explicit collaborators. Must run inside a tokio runtime.
    pub fn with_deps(cfg: RtConfig, deps: RealtimeDeps) -> Realtime {
        let hub = Hub::new(
            Arc::clone(&deps.clock),
            HubConfig::default(),
            Delivery::new(Arc::new(Counters::default())),
        );
        let inner = Arc::new_cyclic(|this: &Weak<Inner>| Inner {
            gateway: GatewayState::new(this.clone(), &deps),
            relay: RelayMetrics::new(Arc::clone(&deps.clock)),
            coalescer: Mutex::new(Coalescer::default()),
            hub,
            cfg,
            deps,
            this: this.clone(),
        });
        Realtime { inner }
    }

    pub fn config(&self) -> &RtConfig {
        &self.inner.cfg
    }

    pub fn clock(&self) -> &Arc<dyn Clock> {
        &self.inner.deps.clock
    }

    pub fn hub(&self) -> &Arc<Hub> {
        &self.inner.hub
    }

    pub fn counters(&self) -> &Arc<Counters> {
        self.inner.hub.delivery().counters()
    }

    pub fn relay_metrics(&self) -> &RelayMetrics {
        &self.inner.relay
    }

    /// `publishWorkspaceRealtimeEvent`. Validates and queues at call time;
    /// the future resolves once the record is in the buffer.
    pub fn publish_workspace_event(
        &self,
        workspace_id: &str,
        event: Envelope,
    ) -> BoxFuture<'static, Result<(), PublishError>> {
        if let Err(err) = validate_workspace_envelope(&event) {
            tracing::warn!(
                err = %err,
                workspace_id = %workspace_id,
                "Rejected invalid workspace realtime event"
            );
            return Box::pin(std::future::ready(Err(
                PublishError::InvalidWorkspaceEvent(err),
            )));
        }
        self.inner.relay.flush(false);
        let event_json = to_json(&event);
        self.inner
            .relay
            .record_event(event.event_type.as_str(), event_json.len());
        let mut done = self.inner.enqueue(workspace_id, event, event_json);
        Box::pin(async move {
            // A dropped sender (the handle went away) also ends the wait.
            let _ = done.wait_for(|done| *done).await;
            Ok(())
        })
    }

    /// `queueWorkspaceRealtimeEvent`: logs and swallows a `PublishError`.
    pub fn queue_workspace_event(
        &self,
        workspace_id: &str,
        event: Envelope,
    ) -> BoxFuture<'static, ()> {
        let event_type: EventType = event.event_type;
        let workspace = workspace_id.to_owned();
        let delivery = self.publish_workspace_event(workspace_id, event);
        Box::pin(async move {
            if let Err(err) = delivery.await {
                tracing::error!(
                    err = %err,
                    event_type = %event_type,
                    workspace_id = %workspace,
                    "Failed to publish realtime event"
                );
            }
        })
    }

    /// `queueMessageCreatedEvent`.
    pub fn queue_message_created(
        &self,
        workspace_id: &str,
        message: Value,
        conversation: Option<&ConversationRoute>,
        inbox_id: Option<&str>,
    ) -> BoxFuture<'static, ()> {
        self.queue_workspace_event(
            workspace_id,
            Envelope::routed(
                EventType::MessageCreated,
                message,
                route_for_conversation(conversation, inbox_id),
            ),
        )
    }

    /// `publishWorkspaceMemberRealtimeEvent`: a direct `member-send` record.
    pub fn publish_member_event(
        &self,
        workspace_id: &str,
        user_id: &str,
        event: Envelope,
    ) -> Result<(), PublishError> {
        self.inner
            .hub
            .append_member_send(workspace_id, user_id, event);
        Ok(())
    }

    /// `publishGuestRealtimeEvent`: a direct `guest-event` record (guest
    /// types only).
    pub fn publish_guest_event(
        &self,
        workspace_id: &str,
        guest_conversation_id: &str,
        event: Envelope,
    ) -> Result<(), PublishError> {
        if let Err(err) = validate_guest_envelope(&event) {
            tracing::warn!(
                err = %err,
                workspace_id = %workspace_id,
                "Rejected invalid guest realtime event"
            );
            return Err(PublishError::InvalidGuestEvent(err));
        }
        self.inner
            .hub
            .append_guest_event(workspace_id, guest_conversation_id, event);
        Ok(())
    }

    /// `revokeWorkspaceMemberRealtimeConnections`: raises the marker, then
    /// closes the member's live sockets minted before `revoked_at_ms`;
    /// 3 attempts with 250 ms linear backoff.
    pub async fn revoke_member(&self, args: RevokeArgs) -> Result<(), RevokeError> {
        let revoked_at_ms = args.revoked_at_ms.unwrap_or_else(|| self.inner.now_ms());
        let mut attempt = 1;
        loop {
            match self
                .inner
                .deps
                .revoke_store
                .raise_marker(&args.workspace_id, &args.user_id, revoked_at_ms)
                .await
            {
                Ok(()) => {
                    self.inner.hub.delivery().revoke_member(
                        &args.workspace_id,
                        &args.user_id,
                        args.reason,
                        revoked_at_ms,
                    );
                    return Ok(());
                }
                Err(err) if attempt >= REVOKE_RETRY_ATTEMPTS => {
                    return Err(RevokeError { source: err });
                }
                Err(err) => {
                    tracing::warn!(
                        err = %err,
                        attempt,
                        workspace_id = %args.workspace_id,
                        "Realtime member revoke attempt failed"
                    );
                    tokio::time::sleep(Duration::from_millis(
                        REVOKE_RETRY_DELAY_MS * u64::from(attempt),
                    ))
                    .await;
                    attempt += 1;
                }
            }
        }
    }

    /// `tryRevokeWorkspaceMemberRealtimeConnections`: never fails; a failed
    /// revoke is logged with `error_message` and handed to
    /// [`RevokeRetry::enqueue`]. Returns `true` when the revoke succeeded.
    pub async fn try_revoke_member(&self, args: RevokeArgs, error_message: &str) -> bool {
        let args = RevokeArgs {
            revoked_at_ms: Some(args.revoked_at_ms.unwrap_or_else(|| self.inner.now_ms())),
            ..args
        };
        match self.revoke_member(args.clone()).await {
            Ok(()) => true,
            Err(err) => {
                tracing::error!(
                    err = %err,
                    user_id = %args.user_id,
                    workspace_id = %args.workspace_id,
                    "{error_message}"
                );
                if let Err(err) = self.inner.deps.revoke_retry.enqueue(&args).await {
                    tracing::error!(
                        err = %err,
                        user_id = %args.user_id,
                        workspace_id = %args.workspace_id,
                        "Failed to enqueue realtime member connection revoke retry"
                    );
                }
                false
            }
        }
    }

    /// `flushAllPendingWorkspaceRealtimeEvents`: appends every pending batch
    /// now and logs the relay window.
    pub async fn flush_all_pending(&self) {
        {
            let mut coalescer = self.inner.coalescer();
            let workspace_ids: Vec<String> = coalescer.batches.keys().cloned().collect();
            for workspace_id in workspace_ids {
                self.inner.flush_locked(&mut coalescer, &workspace_id);
            }
        }
        self.inner.relay.flush(true);
    }

    /// Number of workspaces with a pending (not yet appended) batch.
    pub fn pending_batches(&self) -> usize {
        self.inner.coalescer().batches.len()
    }

    /// Idempotent shutdown: flush producers, wait the drain, close every
    /// socket 4003 `{"retryAfter":..}`, stop timers.
    pub async fn shutdown(&self) {
        if !self.inner.gateway.begin_shutdown() {
            return;
        }
        tracing::info!("Stopping realtime gateway");
        self.inner.gateway.stop_presence();
        self.flush_all_pending().await;
        tokio::time::sleep(Duration::from_millis(self.inner.cfg.shutdown_drain_ms)).await;
        self.inner
            .hub
            .delivery()
            .close_all(next_overload_retry_after_ms());
        self.inner.gateway.log_metrics(&self.inner);
    }

    /// `false` once shutdown started.
    pub fn is_ready(&self) -> bool {
        !self.inner.gateway.is_stopping()
    }
}

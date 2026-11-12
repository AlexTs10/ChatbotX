//! Per-socket delivery rules (spec 2.2, 3.3; `apps/realtime/src/delivery.ts`)
//! and the registry of open sockets.
//!
//! A socket is a [`SocketSink`] (production: the connection task's
//! outbound queue; tests: a recording fake) plus its claims and a `closed`
//! flag. Live entries reach a socket through its connection task, which
//! calls [`Delivery::deliver_member`] / [`Delivery::deliver_guest`];
//! [`Delivery::dispatch`] runs the same rules over every registered socket.
//! Revoke, resync and shutdown closes go through the registry.

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crate::BoxError;
use crate::hub::{Entry, EntryKind};
use crate::protocol::{Envelope, RevokeReason, close, encode_batch, reason, retry_after_reason};
use crate::token::ChatScope;

/// The result of one queued frame (`uWS` `send`: 1 = sent, 2 = dropped).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SendStatus {
    /// Queued for the peer.
    Sent,
    /// Over the slow-consumer budget: the frame was dropped.
    Dropped,
}

/// The write side of one socket.
pub trait SocketSink: Send + Sync {
    /// Queues one text frame. `Err` = the write failed.
    fn send(&self, frame: &str) -> Result<SendStatus, BoxError>;
    /// Closes the socket with `code` and `reason`.
    fn end(&self, code: u16, reason: &str);
}

/// Gateway counters (`RealtimeServerCounters`), logged every heartbeat tick.
#[derive(Debug, Default)]
pub struct Counters {
    pub drops: AtomicU64,
    pub overload_closes: AtomicU64,
    pub records: AtomicU64,
    pub send_bytes: AtomicU64,
    pub send_failures: AtomicU64,
    pub sends: AtomicU64,
    pub token_rejections: AtomicU64,
    pub upgrades: AtomicU64,
}

impl Counters {
    /// Reads one counter.
    pub fn get(counter: &AtomicU64) -> u64 {
        counter.load(Ordering::Relaxed)
    }

    fn add(counter: &AtomicU64, n: u64) {
        counter.fetch_add(n, Ordering::Relaxed);
    }

    /// Reads and resets every counter (one metrics window).
    pub fn take(&self) -> CountersSnapshot {
        let take = |counter: &AtomicU64| counter.swap(0, Ordering::Relaxed);
        CountersSnapshot {
            drops: take(&self.drops),
            overload_closes: take(&self.overload_closes),
            records: take(&self.records),
            send_bytes: take(&self.send_bytes),
            send_failures: take(&self.send_failures),
            sends: take(&self.sends),
            token_rejections: take(&self.token_rejections),
            upgrades: take(&self.upgrades),
        }
    }
}

/// One metrics window of [`Counters`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CountersSnapshot {
    pub drops: u64,
    pub overload_closes: u64,
    pub records: u64,
    pub send_bytes: u64,
    pub send_failures: u64,
    pub sends: u64,
    pub token_rejections: u64,
    pub upgrades: u64,
}

/// The claims a member socket was admitted with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemberSocketData {
    pub workspace_id: String,
    pub user_id: String,
    pub chat_scope: ChatScope,
    pub team_ids: Vec<String>,
    pub iat_ms: u64,
}

/// The claims a guest socket was admitted with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuestSocketData {
    pub workspace_id: String,
    pub guest_conversation_id: String,
}

/// One member socket.
pub struct MemberSocket {
    id: u64,
    data: MemberSocketData,
    closed: AtomicBool,
    sink: Arc<dyn SocketSink>,
}

/// One guest socket.
pub struct GuestSocket {
    id: u64,
    data: GuestSocketData,
    closed: AtomicBool,
    sink: Arc<dyn SocketSink>,
}

static NEXT_SOCKET_ID: AtomicU64 = AtomicU64::new(1);

fn next_socket_id() -> u64 {
    NEXT_SOCKET_ID.fetch_add(1, Ordering::Relaxed)
}

/// Marks `closed` and ends the sink once.
fn close_once(closed: &AtomicBool, sink: &dyn SocketSink, code: u16, reason: &str) -> bool {
    if closed.swap(true, Ordering::SeqCst) {
        return false;
    }
    sink.end(code, reason);
    true
}

impl MemberSocket {
    pub fn new(data: MemberSocketData, sink: Arc<dyn SocketSink>) -> Arc<MemberSocket> {
        Arc::new(MemberSocket {
            id: next_socket_id(),
            data,
            closed: AtomicBool::new(false),
            sink,
        })
    }

    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn data(&self) -> &MemberSocketData {
        &self.data
    }

    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    /// Marks the socket closed without a close frame (the peer is gone).
    pub fn mark_closed(&self) {
        self.closed.store(true, Ordering::SeqCst);
    }

    /// The first call marks the socket closed and ends it with
    /// `code`/`reason`; later calls do nothing. Returns `true` on the first.
    pub fn close(&self, code: u16, reason: &str) -> bool {
        close_once(&self.closed, &*self.sink, code, reason)
    }
}

impl GuestSocket {
    pub fn new(data: GuestSocketData, sink: Arc<dyn SocketSink>) -> Arc<GuestSocket> {
        Arc::new(GuestSocket {
            id: next_socket_id(),
            data,
            closed: AtomicBool::new(false),
            sink,
        })
    }

    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn data(&self) -> &GuestSocketData {
        &self.data
    }

    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    /// Marks the socket closed without a close frame (the peer is gone).
    pub fn mark_closed(&self) {
        self.closed.store(true, Ordering::SeqCst);
    }

    /// Same as [`MemberSocket::close`].
    pub fn close(&self, code: u16, reason: &str) -> bool {
        close_once(&self.closed, &*self.sink, code, reason)
    }
}

/// The sockets of one workspace.
#[derive(Default)]
struct WorkspaceSockets {
    /// user id -> sockets.
    members: HashMap<String, Vec<Arc<MemberSocket>>>,
    guests: Vec<Arc<GuestSocket>>,
}

impl WorkspaceSockets {
    fn is_empty(&self) -> bool {
        self.members.is_empty() && self.guests.is_empty()
    }
}

/// The registry of open sockets plus the delivery rules.
pub struct Delivery {
    counters: Arc<Counters>,
    registry: Mutex<HashMap<String, WorkspaceSockets>>,
}

/// `hasRouteMatch`.
fn route_matches(event: &Envelope, user_id: &str, team_ids: &[String]) -> bool {
    match &event.route {
        None => true,
        Some(route) => {
            route.assigned_user_ids.iter().any(|id| id == user_id)
                || route
                    .assigned_team_ids
                    .iter()
                    .any(|team| team_ids.contains(team))
        }
    }
}

/// The frame a member socket gets for one entry, if any.
fn member_frame<'a>(data: &MemberSocketData, entry: &'a Entry) -> Option<Cow<'a, str>> {
    if entry.workspace_id != data.workspace_id {
        return None;
    }
    match &entry.kind {
        EntryKind::WorkspaceEvents { events, all_frame } => match data.chat_scope {
            ChatScope::All => Some(Cow::Borrowed(&**all_frame)),
            ChatScope::None => None,
            ChatScope::Assigned => {
                let filtered: Vec<&Envelope> = events
                    .iter()
                    .filter(|event| route_matches(event, &data.user_id, &data.team_ids))
                    .collect();
                if filtered.is_empty() {
                    None
                } else if filtered.len() == events.len() {
                    Some(Cow::Borrowed(&**all_frame))
                } else {
                    Some(Cow::Owned(encode_batch(&filtered, &entry.seq.to_string())))
                }
            }
        },
        EntryKind::MemberSend { user_id, frame, .. } => {
            (*user_id == data.user_id).then(|| Cow::Borrowed(&**frame))
        }
        EntryKind::GuestEvent { .. } => None,
    }
}

/// The frame a guest socket gets for one entry, if any.
fn guest_frame<'a>(data: &GuestSocketData, entry: &'a Entry) -> Option<&'a str> {
    match &entry.kind {
        EntryKind::GuestEvent {
            guest_conversation_id,
            frame,
            ..
        } if entry.workspace_id == data.workspace_id
            && *guest_conversation_id == data.guest_conversation_id =>
        {
            Some(frame)
        }
        _ => None,
    }
}

impl Delivery {
    pub fn new(counters: Arc<Counters>) -> Delivery {
        Delivery {
            counters,
            registry: Mutex::new(HashMap::new()),
        }
    }

    pub fn counters(&self) -> &Arc<Counters> {
        &self.counters
    }

    fn registry(&self) -> MutexGuard<'_, HashMap<String, WorkspaceSockets>> {
        self.registry.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// `recordSend` + `sendWithResync`: a dropped frame closes 4002
    /// `backpressure-drop`, a failed one 4002 `send-failed`.
    fn send(&self, closed: &AtomicBool, sink: &dyn SocketSink, workspace_id: &str, frame: &str) {
        let bytes = frame.len() as u64;
        match sink.send(frame) {
            Ok(SendStatus::Sent) => {
                Counters::add(&self.counters.sends, 1);
                Counters::add(&self.counters.send_bytes, bytes);
            }
            Ok(SendStatus::Dropped) => {
                Counters::add(&self.counters.sends, 1);
                Counters::add(&self.counters.send_bytes, bytes);
                Counters::add(&self.counters.drops, 1);
                close_once(closed, sink, close::RESYNC, reason::BACKPRESSURE_DROP);
            }
            Err(err) => {
                Counters::add(&self.counters.send_failures, 1);
                tracing::warn!(err = %err, workspace_id = %workspace_id, "Realtime send failed");
                close_once(closed, sink, close::RESYNC, reason::SEND_FAILED);
            }
        }
    }

    /// Registers a member socket. Returns `true` when it is the user's first
    /// socket in the workspace.
    pub fn add_workspace_socket(&self, socket: &Arc<MemberSocket>) -> bool {
        let mut registry = self.registry();
        let sockets = registry
            .entry(socket.data.workspace_id.clone())
            .or_default()
            .members
            .entry(socket.data.user_id.clone())
            .or_default();
        let first = sockets.is_empty();
        if !sockets.iter().any(|known| known.id == socket.id) {
            sockets.push(Arc::clone(socket));
        }
        first
    }

    /// Removes a member socket. Returns `true` when it was the user's last.
    pub fn remove_workspace_socket(&self, socket: &MemberSocket) -> bool {
        let mut registry = self.registry();
        let Some(workspace) = registry.get_mut(&socket.data.workspace_id) else {
            return false;
        };
        let Some(sockets) = workspace.members.get_mut(&socket.data.user_id) else {
            return false;
        };
        let before = sockets.len();
        sockets.retain(|known| known.id != socket.id);
        let removed = sockets.len() < before;
        let last = removed && sockets.is_empty();
        if sockets.is_empty() {
            workspace.members.remove(&socket.data.user_id);
        }
        if workspace.is_empty() {
            registry.remove(&socket.data.workspace_id);
        }
        last
    }

    pub fn add_guest_socket(&self, socket: &Arc<GuestSocket>) {
        let mut registry = self.registry();
        let guests = &mut registry
            .entry(socket.data.workspace_id.clone())
            .or_default()
            .guests;
        if !guests.iter().any(|known| known.id == socket.id) {
            guests.push(Arc::clone(socket));
        }
    }

    pub fn remove_guest_socket(&self, socket: &GuestSocket) {
        let mut registry = self.registry();
        let Some(workspace) = registry.get_mut(&socket.data.workspace_id) else {
            return;
        };
        workspace.guests.retain(|known| known.id != socket.id);
        if workspace.is_empty() {
            registry.remove(&socket.data.workspace_id);
        }
    }

    /// The user ids with at least one open member socket in `workspace_id`.
    pub fn connected_user_ids(&self, workspace_id: &str) -> Vec<String> {
        self.registry()
            .get(workspace_id)
            .map(|workspace| workspace.members.keys().cloned().collect())
            .unwrap_or_default()
    }

    /// Workspaces with at least one registered member or guest socket.
    pub fn connected_workspace_ids(&self) -> Vec<String> {
        self.registry().keys().cloned().collect()
    }

    /// Every member socket of one workspace (one user when `user_id` is set).
    fn member_sockets(&self, workspace_id: &str, user_id: Option<&str>) -> Vec<Arc<MemberSocket>> {
        let registry = self.registry();
        let Some(workspace) = registry.get(workspace_id) else {
            return Vec::new();
        };
        match user_id {
            Some(user_id) => workspace.members.get(user_id).cloned().unwrap_or_default(),
            None => workspace.members.values().flatten().cloned().collect(),
        }
    }

    fn guest_sockets(&self, workspace_id: &str) -> Vec<Arc<GuestSocket>> {
        self.registry()
            .get(workspace_id)
            .map(|workspace| workspace.guests.clone())
            .unwrap_or_default()
    }

    /// Live fan-out of one entry to every registered socket of its
    /// workspace, with the same rules as the connection tasks.
    pub fn dispatch(&self, entry: &Entry) {
        match &entry.kind {
            EntryKind::WorkspaceEvents { .. } => {
                for socket in self.member_sockets(&entry.workspace_id, None) {
                    self.deliver_member(&socket, entry);
                }
            }
            EntryKind::MemberSend { user_id, .. } => {
                for socket in self.member_sockets(&entry.workspace_id, Some(user_id)) {
                    self.deliver_member(&socket, entry);
                }
            }
            EntryKind::GuestEvent { .. } => {
                for socket in self.guest_sockets(&entry.workspace_id) {
                    self.deliver_guest(&socket, entry);
                }
            }
        }
    }

    /// The live rule for one member socket: a workspace entry is filtered
    /// by scope (`all`: the shared frame; `assigned`: by route; `none`:
    /// nothing); a member-send reaches only its user; guest entries never.
    pub fn deliver_member(&self, socket: &MemberSocket, entry: &Entry) {
        if socket.is_closed() {
            return;
        }
        if let Some(frame) = member_frame(&socket.data, entry) {
            self.send(
                &socket.closed,
                &*socket.sink,
                &socket.data.workspace_id,
                &frame,
            );
        }
    }

    /// The live rule for one guest socket: only guest events of its
    /// `(workspace, guest conversation)`.
    pub fn deliver_guest(&self, socket: &GuestSocket, entry: &Entry) {
        if socket.is_closed() {
            return;
        }
        if let Some(frame) = guest_frame(&socket.data, entry) {
            self.send(
                &socket.closed,
                &*socket.sink,
                &socket.data.workspace_id,
                frame,
            );
        }
    }

    /// `member-revoke`: closes the user's sockets minted before
    /// `revoked_at_ms` (`iat_ms < revoked_at_ms`): 4001 `revoked` for
    /// `deleted`, 4004 `reauth` for `reauth`. Returns the closed count.
    pub fn revoke_member(
        &self,
        workspace_id: &str,
        user_id: &str,
        reason: RevokeReason,
        revoked_at_ms: u64,
    ) -> usize {
        let (code, text) = match reason {
            RevokeReason::Deleted => (close::REVOKED, reason::REVOKED),
            RevokeReason::Reauth => (close::REAUTH, reason::REAUTH),
        };
        self.member_sockets(workspace_id, Some(user_id))
            .iter()
            .filter(|socket| socket.data.iat_ms < revoked_at_ms)
            .filter(|socket| socket.close(code, text))
            .count()
    }

    /// Closes 4002 `reason` every member and guest socket of one workspace.
    pub fn resync_workspace(&self, workspace_id: &str, reason: &str) {
        for socket in self.member_sockets(workspace_id, None) {
            socket.close(close::RESYNC, reason);
        }
        for socket in self.guest_sockets(workspace_id) {
            socket.close(close::RESYNC, reason);
        }
    }

    /// Closes every socket 4003 `{"retryAfter":<ms>}` (shutdown).
    pub fn close_all(&self, retry_after_ms: u64) {
        let text = retry_after_reason(retry_after_ms);
        let (members, guests): (Vec<_>, Vec<_>) = {
            let registry = self.registry();
            (
                registry
                    .values()
                    .flat_map(|workspace| workspace.members.values().flatten().cloned())
                    .collect(),
                registry
                    .values()
                    .flat_map(|workspace| workspace.guests.iter().cloned())
                    .collect(),
            )
        };
        for socket in members {
            socket.close(close::OVERLOADED, &text);
        }
        for socket in guests {
            socket.close(close::OVERLOADED, &text);
        }
    }

    /// Replays `entries` to a member socket with the live rules (member
    /// sends of other users skipped). Stops at the first close; returns
    /// `false` when the socket is closed.
    pub fn replay_workspace_socket(&self, socket: &MemberSocket, entries: &[Arc<Entry>]) -> bool {
        for entry in entries {
            if socket.is_closed() {
                return false;
            }
            self.deliver_member(socket, entry);
        }
        !socket.is_closed()
    }

    /// Replays `entries` to a guest socket (only its own guest events).
    pub fn replay_guest_socket(&self, socket: &GuestSocket, entries: &[Arc<Entry>]) -> bool {
        for entry in entries {
            if socket.is_closed() {
                return false;
            }
            self.deliver_guest(socket, entry);
        }
        !socket.is_closed()
    }

    /// Sends `{"batch":[],"seq":"<seq>"}`; nothing once the socket is closed.
    pub fn send_cursor(&self, socket: &MemberSocket, seq: &str) {
        if socket.is_closed() {
            return;
        }
        self.send(
            &socket.closed,
            &*socket.sink,
            &socket.data.workspace_id,
            &encode_batch(&[], seq),
        );
    }
}

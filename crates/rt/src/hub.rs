//! The hub (spec 3.3): one replay buffer and one `broadcast` channel per
//! workspace, under one lock with the seq allocation.
//!
//! A connecting socket takes its replay snapshot, subscribes and registers
//! under the same lock, so replay and live never overlap or leave a gap.
//! Buffers are bounded by count, bytes and age; a maintenance task runs
//! only while some buffer is non-empty.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::Duration;

use chatbotx_kv::Clock;
use tokio::sync::broadcast;

use crate::delivery::{Delivery, GuestSocket, MemberSocket};
use crate::protocol::{Envelope, Seq, reason, to_json};

/// Entries kept per workspace (replay refuses more than 500 own entries).
pub const MAX_BUFFER_ENTRIES: usize = 512;
/// Age bound of a buffered entry (the old `XTRIM MINID ~ now - 300 000`).
pub const MAX_BUFFER_AGE_MS: i64 = 300_000;
/// Byte bound of one workspace buffer.
pub const MAX_BUFFER_BYTES: usize = 4 * 1024 * 1024;
/// `broadcast` capacity per workspace; a receiver this far behind lags.
pub const CHANNEL_CAPACITY: usize = 256;
/// `MAX_REPLAY_ENTRIES`: more own entries after the cursor -> resync.
pub const MAX_REPLAY_ENTRIES: usize = 500;
/// The buffer maintenance tick (runs only while a buffer is non-empty).
pub const MAINTENANCE_INTERVAL_MS: u64 = 60_000;

/// Bounds of the hub.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HubConfig {
    pub max_entries: usize,
    pub max_age_ms: i64,
    pub max_bytes: usize,
    pub channel_capacity: usize,
}

impl Default for HubConfig {
    fn default() -> HubConfig {
        HubConfig {
            max_entries: MAX_BUFFER_ENTRIES,
            max_age_ms: MAX_BUFFER_AGE_MS,
            max_bytes: MAX_BUFFER_BYTES,
            channel_capacity: CHANNEL_CAPACITY,
        }
    }
}

/// One delivered record.
#[derive(Debug)]
pub struct Entry {
    pub seq: Seq,
    pub workspace_id: String,
    pub kind: EntryKind,
    /// Append time (`Clock::now_ms`), for the age bound.
    pub at_ms: i64,
    /// Frame bytes, for the byte bound.
    pub bytes: usize,
}

/// The three buffered record kinds (revoke and resync never enter it).
#[derive(Debug)]
pub enum EntryKind {
    /// `workspace-events`; `all_frame` = the frame for `chatScope: all`.
    WorkspaceEvents {
        events: Vec<Envelope>,
        all_frame: Arc<str>,
    },
    /// `member-send`.
    MemberSend {
        user_id: String,
        event: Envelope,
        frame: Arc<str>,
    },
    /// `guest-event`.
    GuestEvent {
        guest_conversation_id: String,
        event: Envelope,
        frame: Arc<str>,
    },
}

/// A record before its seq is known: the kind plus the serialized events
/// (built outside the hub lock).
enum Body {
    WorkspaceEvents {
        events: Vec<Envelope>,
        events_json: String,
    },
    MemberSend {
        user_id: String,
        event: Envelope,
        event_json: String,
    },
    GuestEvent {
        guest_conversation_id: String,
        event: Envelope,
        event_json: String,
    },
}

/// `{"batch":[<events_json>],"seq":"<seq>"}`.
fn frame(events_json: &str, seq: Seq) -> Arc<str> {
    Arc::from(format!(r#"{{"batch":[{events_json}],"seq":"{seq}"}}"#))
}

impl Body {
    fn workspace_events(events: Vec<Envelope>) -> Body {
        let events_json = events.iter().map(to_json).collect::<Vec<_>>().join(",");
        Body::WorkspaceEvents {
            events,
            events_json,
        }
    }

    fn member_send(user_id: &str, event: Envelope) -> Body {
        Body::MemberSend {
            user_id: user_id.to_owned(),
            event_json: to_json(&event),
            event,
        }
    }

    fn guest_event(guest_conversation_id: &str, event: Envelope) -> Body {
        Body::GuestEvent {
            guest_conversation_id: guest_conversation_id.to_owned(),
            event_json: to_json(&event),
            event,
        }
    }

    fn into_entry(self, seq: Seq, workspace_id: &str, at_ms: i64) -> Arc<Entry> {
        let (kind, bytes) = match self {
            Body::WorkspaceEvents {
                events,
                events_json,
            } => {
                let all_frame = frame(&events_json, seq);
                let bytes = all_frame.len();
                (EntryKind::WorkspaceEvents { events, all_frame }, bytes)
            }
            Body::MemberSend {
                user_id,
                event,
                event_json,
            } => {
                let frame = frame(&event_json, seq);
                let bytes = frame.len();
                (
                    EntryKind::MemberSend {
                        user_id,
                        event,
                        frame,
                    },
                    bytes,
                )
            }
            Body::GuestEvent {
                guest_conversation_id,
                event,
                event_json,
            } => {
                let frame = frame(&event_json, seq);
                let bytes = frame.len();
                (
                    EntryKind::GuestEvent {
                        guest_conversation_id,
                        event,
                        frame,
                    },
                    bytes,
                )
            }
        };
        Arc::new(Entry {
            seq,
            workspace_id: workspace_id.to_owned(),
            kind,
            at_ms,
            bytes,
        })
    }
}

impl Entry {
    /// A `workspace-events` entry (frame built here).
    pub fn workspace_events(seq: Seq, workspace_id: &str, events: Vec<Envelope>) -> Arc<Entry> {
        Body::workspace_events(events).into_entry(seq, workspace_id, 0)
    }

    /// A `member-send` entry.
    pub fn member_send(seq: Seq, workspace_id: &str, user_id: &str, event: Envelope) -> Arc<Entry> {
        Body::member_send(user_id, event).into_entry(seq, workspace_id, 0)
    }

    /// A `guest-event` entry.
    pub fn guest_event(
        seq: Seq,
        workspace_id: &str,
        guest_conversation_id: &str,
        event: Envelope,
    ) -> Arc<Entry> {
        Body::guest_event(guest_conversation_id, event).into_entry(seq, workspace_id, 0)
    }
}

/// The replay snapshot of one connect (spec 2.3).
#[derive(Debug, Clone)]
pub struct Replay {
    /// A 4002 reason; then `entries` is empty.
    pub close_reason: Option<&'static str>,
    pub entries: Vec<Arc<Entry>>,
    /// The cursor frame value: the newest replayed id, else `lastSeq`, else
    /// the newest buffered id (`0-0` for an empty buffer).
    pub cursor: Seq,
}

impl Replay {
    fn closed(reason: &'static str) -> Replay {
        Replay {
            close_reason: Some(reason),
            entries: Vec::new(),
            cursor: Seq::ZERO,
        }
    }
}

struct WorkspaceBus {
    buffer: VecDeque<Arc<Entry>>,
    buffer_bytes: usize,
    /// The newest evicted seq: a cursor at or after it lost nothing.
    evicted_up_to: Option<Seq>,
    /// The newest seq ever appended (kept after eviction).
    last_seq: Option<Seq>,
    tx: broadcast::Sender<Arc<Entry>>,
    /// Open sockets + admitted pending upgrades.
    refs: usize,
}

impl WorkspaceBus {
    fn new(capacity: usize, horizon: Option<Seq>) -> WorkspaceBus {
        WorkspaceBus {
            buffer: VecDeque::new(),
            buffer_bytes: 0,
            evicted_up_to: horizon,
            last_seq: horizon,
            tx: broadcast::channel(capacity.max(1)).0,
            refs: 0,
        }
    }

    fn evict_front(&mut self) {
        if let Some(entry) = self.buffer.pop_front() {
            self.buffer_bytes -= entry.bytes;
            self.evicted_up_to = Some(self.evicted_up_to.map_or(entry.seq, |s| s.max(entry.seq)));
        }
    }

    /// Drops entries older than the age bound.
    fn prune_aged(&mut self, now_ms: i64, cfg: &HubConfig) {
        let cutoff = now_ms.saturating_sub(cfg.max_age_ms);
        while self
            .buffer
            .front()
            .is_some_and(|entry| entry.at_ms < cutoff)
        {
            self.evict_front();
        }
    }

    fn push(&mut self, entry: Arc<Entry>, now_ms: i64, cfg: &HubConfig) {
        self.buffer_bytes += entry.bytes;
        self.last_seq = Some(entry.seq);
        self.buffer.push_back(entry);
        self.prune_aged(now_ms, cfg);
        while self.buffer.len() > cfg.max_entries.max(1)
            || (self.buffer.len() > 1 && self.buffer_bytes > cfg.max_bytes)
        {
            self.evict_front();
        }
    }

    /// `hasRecentEntriesGap`.
    fn has_gap(&self, cursor: Seq) -> bool {
        match self.evicted_up_to {
            Some(evicted) => evicted > cursor,
            None => self
                .buffer
                .front()
                .is_some_and(|oldest| oldest.seq > cursor),
        }
    }

    /// The replay rules of spec 2.3.
    fn replay(&self, last_seq: Option<&str>) -> Replay {
        let newest = self.last_seq.unwrap_or(Seq::ZERO);
        let Some(raw) = last_seq.filter(|raw| !raw.is_empty()) else {
            return Replay {
                close_reason: None,
                entries: Vec::new(),
                cursor: newest,
            };
        };
        let Ok(cursor) = raw.parse::<Seq>() else {
            return Replay::closed(reason::INVALID_LAST_SEQ);
        };
        if self.has_gap(cursor) {
            return Replay::closed(reason::REPLAY_WINDOW_EXPIRED);
        }
        let ahead = match self.last_seq {
            None => cursor != Seq::ZERO,
            Some(last) => cursor > last,
        };
        if ahead {
            return Replay::closed(reason::REPLAY_CURSOR_AHEAD);
        }
        let start = self.buffer.partition_point(|entry| entry.seq <= cursor);
        if self.buffer.len() - start > MAX_REPLAY_ENTRIES {
            return Replay::closed(reason::REPLAY_WINDOW_TOO_LARGE);
        }
        let entries: Vec<Arc<Entry>> = self.buffer.range(start..).cloned().collect();
        Replay {
            close_reason: None,
            cursor: entries.last().map_or(cursor, |entry| entry.seq),
            entries,
        }
    }
}

#[derive(Default)]
struct HubState {
    last_seq: Option<Seq>,
    buses: HashMap<String, WorkspaceBus>,
    /// The `last_seq` of removed buses, so a reconnect after the removal
    /// keeps its cursor instead of a false `replay-cursor-ahead`.
    horizons: HashMap<String, Seq>,
    maintenance_running: bool,
}

impl HubState {
    /// `ms = max(now, last.ms)`, `n = last.n + 1` on the same ms, else 0.
    fn next_seq(&mut self, now_ms: i64) -> Seq {
        let now = u64::try_from(now_ms).unwrap_or(0);
        let seq = match self.last_seq {
            Some(last) if now <= last.ms => Seq::new(last.ms, last.n + 1),
            _ => Seq::new(now, 0),
        };
        let seq = if seq == Seq::ZERO {
            Seq::new(0, 1)
        } else {
            seq
        };
        self.last_seq = Some(seq);
        seq
    }

    fn bus_mut(&mut self, workspace_id: &str, capacity: usize) -> &mut WorkspaceBus {
        if !self.buses.contains_key(workspace_id) {
            let horizon = self.horizons.remove(workspace_id);
            self.buses.insert(
                workspace_id.to_owned(),
                WorkspaceBus::new(capacity, horizon),
            );
        }
        self.buses
            .get_mut(workspace_id)
            .unwrap_or_else(|| unreachable!("inserted above"))
    }

    /// Removes the bus when it holds nothing and nobody references it.
    fn drop_if_idle(&mut self, workspace_id: &str) {
        let idle = self
            .buses
            .get(workspace_id)
            .is_some_and(|bus| bus.refs == 0 && bus.buffer.is_empty());
        if !idle {
            return;
        }
        if let Some(last) = self.buses.remove(workspace_id).and_then(|bus| bus.last_seq) {
            self.horizons.insert(workspace_id.to_owned(), last);
        }
    }
}

/// The hub. Never hold its lock across `.await`.
pub struct Hub {
    clock: Arc<dyn Clock>,
    cfg: HubConfig,
    delivery: Delivery,
    state: Mutex<HubState>,
    this: Weak<Hub>,
}

/// A reference that keeps a workspace bus alive (pending upgrade or open
/// socket). Dropping it releases the reference.
pub struct WorkspaceRef {
    hub: Arc<Hub>,
    workspace_id: String,
}

impl WorkspaceRef {
    pub fn workspace_id(&self) -> &str {
        &self.workspace_id
    }

    pub fn hub(&self) -> &Arc<Hub> {
        &self.hub
    }
}

impl Drop for WorkspaceRef {
    fn drop(&mut self) {
        let mut state = self.hub.lock();
        if let Some(bus) = state.buses.get_mut(&self.workspace_id) {
            bus.refs = bus.refs.saturating_sub(1);
        }
        state.drop_if_idle(&self.workspace_id);
    }
}

/// The registration of one socket in the hub. Dropping it unregisters the
/// socket (an aborted upgrade or a closed connection).
pub struct Registration {
    hub: Arc<Hub>,
    socket: RegisteredSocket,
}

enum RegisteredSocket {
    Member(Arc<MemberSocket>),
    Guest(Arc<GuestSocket>),
    Done,
}

impl Registration {
    /// Unregisters now. Returns `true` when this was the member's last
    /// socket in the workspace (`false` for guests).
    pub fn unregister(&mut self) -> bool {
        match std::mem::replace(&mut self.socket, RegisteredSocket::Done) {
            RegisteredSocket::Member(socket) => self.hub.delivery.remove_workspace_socket(&socket),
            RegisteredSocket::Guest(socket) => {
                self.hub.delivery.remove_guest_socket(&socket);
                false
            }
            RegisteredSocket::Done => false,
        }
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        self.unregister();
    }
}

/// A socket attached to its workspace: replay snapshot, live receiver and
/// registration, all taken under one lock.
pub struct Attached {
    pub replay: Replay,
    pub rx: broadcast::Receiver<Arc<Entry>>,
    /// `true` when this is the user's first member socket in the workspace.
    pub first_user_socket: bool,
    pub registration: Registration,
}

impl Hub {
    pub fn new(clock: Arc<dyn Clock>, cfg: HubConfig, delivery: Delivery) -> Arc<Hub> {
        Arc::new_cyclic(|this| Hub {
            clock,
            cfg,
            delivery,
            state: Mutex::new(HubState::default()),
            this: this.clone(),
        })
    }

    fn lock(&self) -> MutexGuard<'_, HubState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn delivery(&self) -> &Delivery {
        &self.delivery
    }

    pub fn config(&self) -> &HubConfig {
        &self.cfg
    }

    /// Seq, buffer append and broadcast under the lock.
    fn append(&self, workspace_id: &str, body: Body) -> Seq {
        let now = self.clock.now_ms();
        let mut state = self.lock();
        let seq = state.next_seq(now);
        let entry = body.into_entry(seq, workspace_id, now);
        let bus = state.bus_mut(workspace_id, self.cfg.channel_capacity);
        bus.push(Arc::clone(&entry), now, &self.cfg);
        // No receiver is fine: the entry stays in the buffer for replay.
        let _ = bus.tx.send(entry);
        let start_maintenance = !state.maintenance_running;
        if start_maintenance {
            state.maintenance_running = self.spawn_maintenance();
        }
        drop(state);
        self.delivery
            .counters()
            .records
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        seq
    }

    /// Starts the maintenance task when a runtime is there.
    fn spawn_maintenance(&self) -> bool {
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return false;
        };
        let weak = self.this.clone();
        handle.spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(MAINTENANCE_INTERVAL_MS)).await;
                let Some(hub) = weak.upgrade() else {
                    return;
                };
                if !hub.maintenance_tick() {
                    return;
                }
            }
        });
        true
    }

    /// One maintenance pass; stops the task (returns `false`) once every
    /// buffer is empty.
    fn maintenance_tick(&self) -> bool {
        let now = self.clock.now_ms();
        let mut state = self.lock();
        let busy = Self::maintain_locked(&mut state, now, &self.cfg);
        if !busy {
            state.maintenance_running = false;
        }
        busy
    }

    fn maintain_locked(state: &mut HubState, now_ms: i64, cfg: &HubConfig) -> bool {
        let mut idle = Vec::new();
        let mut busy = false;
        for (workspace_id, bus) in &mut state.buses {
            bus.prune_aged(now_ms, cfg);
            busy |= !bus.buffer.is_empty();
            if bus.refs == 0 && bus.buffer.is_empty() {
                idle.push(workspace_id.clone());
            }
        }
        for workspace_id in idle {
            state.drop_if_idle(&workspace_id);
        }
        busy
    }

    /// Appends a `workspace-events` record (seq, buffer, broadcast).
    pub fn append_workspace_events(&self, workspace_id: &str, events: Vec<Envelope>) -> Seq {
        self.append(workspace_id, Body::workspace_events(events))
    }

    /// [`Hub::append_workspace_events`] with the events already serialized
    /// (`events_json` = the comma-joined event JSON).
    pub(crate) fn append_workspace_events_serialized(
        &self,
        workspace_id: &str,
        events: Vec<Envelope>,
        events_json: String,
    ) -> Seq {
        self.append(
            workspace_id,
            Body::WorkspaceEvents {
                events,
                events_json,
            },
        )
    }

    /// Appends a `member-send` record.
    pub fn append_member_send(&self, workspace_id: &str, user_id: &str, event: Envelope) -> Seq {
        self.append(workspace_id, Body::member_send(user_id, event))
    }

    /// Appends a `guest-event` record.
    pub fn append_guest_event(
        &self,
        workspace_id: &str,
        guest_conversation_id: &str,
        event: Envelope,
    ) -> Seq {
        self.append(
            workspace_id,
            Body::guest_event(guest_conversation_id, event),
        )
    }

    /// The replay rules of spec 2.3 against the workspace buffer (no
    /// subscription).
    pub fn load_replay(&self, workspace_id: &str, last_seq: Option<&str>) -> Replay {
        let now = self.clock.now_ms();
        let mut state = self.lock();
        let replay = match state.buses.get_mut(workspace_id) {
            Some(bus) => {
                bus.prune_aged(now, &self.cfg);
                bus.replay(last_seq)
            }
            None => {
                let horizon = state.horizons.get(workspace_id).copied();
                WorkspaceBus::new(1, horizon).replay(last_seq)
            }
        };
        state.drop_if_idle(workspace_id);
        replay
    }

    /// `hasRecentEntriesGap`: whether a client at `cursor` may have missed
    /// entries the buffer no longer holds (an evicted entry after the
    /// cursor, or, with nothing evicted, a retained head after the cursor).
    /// A malformed cursor counts as a gap.
    pub fn has_gap(&self, workspace_id: &str, cursor: &str) -> bool {
        let Ok(cursor) = cursor.parse::<Seq>() else {
            return true;
        };
        let state = self.lock();
        match state.buses.get(workspace_id) {
            Some(bus) => bus.has_gap(cursor),
            None => state
                .horizons
                .get(workspace_id)
                .is_some_and(|horizon| *horizon > cursor),
        }
    }

    /// The buffered entries of one workspace, oldest first.
    pub fn buffered(&self, workspace_id: &str) -> Vec<Arc<Entry>> {
        self.lock()
            .buses
            .get(workspace_id)
            .map(|bus| bus.buffer.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Open sockets + admitted pending upgrades of one workspace.
    pub fn workspace_refs(&self, workspace_id: &str) -> usize {
        self.lock()
            .buses
            .get(workspace_id)
            .map_or(0, |bus| bus.refs)
    }

    /// Number of workspace buses alive.
    pub fn bus_count(&self) -> usize {
        self.lock().buses.len()
    }

    /// Whether the buffer maintenance task is running.
    pub fn maintenance_running(&self) -> bool {
        self.lock().maintenance_running
    }

    /// Takes a workspace reference (creates the bus when needed).
    pub fn retain(self: &Arc<Self>, workspace_id: &str) -> WorkspaceRef {
        let mut state = self.lock();
        state.bus_mut(workspace_id, self.cfg.channel_capacity).refs += 1;
        WorkspaceRef {
            hub: Arc::clone(self),
            workspace_id: workspace_id.to_owned(),
        }
    }

    /// Snapshot + subscribe under the lock, then `register` (nested
    /// delivery lock, still under the hub lock).
    fn attach(
        self: &Arc<Self>,
        workspace_id: &str,
        last_seq: Option<&str>,
        register: impl FnOnce(&Delivery) -> (bool, RegisteredSocket),
    ) -> Attached {
        let now = self.clock.now_ms();
        let mut state = self.lock();
        let bus = state.bus_mut(workspace_id, self.cfg.channel_capacity);
        bus.prune_aged(now, &self.cfg);
        let replay = bus.replay(last_seq);
        let rx = bus.tx.subscribe();
        let (first_user_socket, socket) = register(&self.delivery);
        drop(state);
        Attached {
            replay,
            rx,
            first_user_socket,
            registration: Registration {
                hub: Arc::clone(self),
                socket,
            },
        }
    }

    /// Replay snapshot + subscribe + register of a member socket.
    pub fn attach_member(
        self: &Arc<Self>,
        socket: &Arc<MemberSocket>,
        last_seq: Option<&str>,
    ) -> Attached {
        self.attach(&socket.data().workspace_id, last_seq, |delivery| {
            (
                delivery.add_workspace_socket(socket),
                RegisteredSocket::Member(Arc::clone(socket)),
            )
        })
    }

    /// Replay snapshot + subscribe + register of a guest socket.
    pub fn attach_guest(
        self: &Arc<Self>,
        socket: &Arc<GuestSocket>,
        last_seq: Option<&str>,
    ) -> Attached {
        self.attach(&socket.data().workspace_id, last_seq, |delivery| {
            delivery.add_guest_socket(socket);
            (false, RegisteredSocket::Guest(Arc::clone(socket)))
        })
    }

    /// Drops entries older than the age bound and removes empty buses
    /// without references. Returns `true` while some buffer is non-empty.
    pub fn maintain(&self) -> bool {
        let now = self.clock.now_ms();
        let mut state = self.lock();
        Self::maintain_locked(&mut state, now, &self.cfg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::delivery::Counters as DeliveryCounters;
    use chatbotx_kv::TestClock;
    use serde_json::json;

    fn hub(clock: &Arc<TestClock>, cfg: HubConfig) -> Arc<Hub> {
        Hub::new(
            clock.clone(),
            cfg,
            Delivery::new(Arc::new(DeliveryCounters::default())),
        )
    }

    fn typing() -> Vec<Envelope> {
        vec![serde_json::from_value(json!({ "data": {}, "eventType": "typing" })).unwrap()]
    }

    #[test]
    fn seq_is_monotonic_across_workspaces_and_a_clock_step_back() {
        let clock = Arc::new(TestClock::new(100));
        let hub = hub(&clock, HubConfig::default());
        assert_eq!(hub.append_workspace_events("a", typing()), Seq::new(100, 0));
        assert_eq!(hub.append_workspace_events("b", typing()), Seq::new(100, 1));
        clock.set(50);
        assert_eq!(hub.append_workspace_events("a", typing()), Seq::new(100, 2));
        clock.set(101);
        assert_eq!(hub.append_workspace_events("b", typing()), Seq::new(101, 0));
    }

    #[test]
    fn the_buffer_keeps_the_byte_bound_but_never_drops_the_newest_entry() {
        let clock = Arc::new(TestClock::new(1));
        let cfg = HubConfig {
            max_bytes: 100,
            ..HubConfig::default()
        };
        let hub = hub(&clock, cfg);
        let big: Vec<Envelope> = vec![
            serde_json::from_value(json!({ "data": "x".repeat(200), "eventType": "typing" }))
                .unwrap(),
        ];
        hub.append_workspace_events("a", big.clone());
        let second = hub.append_workspace_events("a", big);
        let buffered = hub.buffered("a");
        assert_eq!(buffered.len(), 1);
        assert_eq!(buffered[0].seq, second);
        assert!(hub.has_gap("a", "0-0"));
        assert!(!hub.has_gap("a", &(Seq::new(second.ms, second.n - 1)).to_string()));
    }

    #[test]
    fn an_idle_bus_is_removed_and_its_horizon_keeps_the_cursor_valid() {
        let clock = Arc::new(TestClock::new(1_000));
        let hub = hub(&clock, HubConfig::default());
        let seq = hub.append_workspace_events("a", typing());
        clock.advance(MAX_BUFFER_AGE_MS + 1);
        assert!(!hub.maintain());
        assert_eq!(hub.bus_count(), 0);

        let replay = hub.load_replay("a", Some(&seq.to_string()));
        assert_eq!(replay.close_reason, None);
        assert_eq!(replay.cursor, seq);
        let older = Seq::new(seq.ms - 1, 0).to_string();
        assert_eq!(
            hub.load_replay("a", Some(&older)).close_reason,
            Some(reason::REPLAY_WINDOW_EXPIRED)
        );
        let ahead = Seq::new(seq.ms + 1, 0).to_string();
        assert_eq!(
            hub.load_replay("a", Some(&ahead)).close_reason,
            Some(reason::REPLAY_CURSOR_AHEAD)
        );
    }

    #[test]
    fn a_referenced_bus_survives_maintenance_and_goes_with_its_last_reference() {
        let clock = Arc::new(TestClock::new(1_000));
        let hub = hub(&clock, HubConfig::default());
        let reference = hub.retain("a");
        assert_eq!(hub.workspace_refs("a"), 1);
        assert!(!hub.maintain());
        assert_eq!(hub.bus_count(), 1);
        drop(reference);
        assert_eq!(hub.workspace_refs("a"), 0);
        assert_eq!(hub.bus_count(), 0);
    }

    #[test]
    fn replay_returns_the_entries_after_the_cursor_and_the_newest_as_cursor() {
        let clock = Arc::new(TestClock::new(10));
        let hub = hub(&clock, HubConfig::default());
        let first = hub.append_workspace_events("a", typing());
        clock.set(20);
        let second = hub.append_workspace_events("a", typing());
        let replay = hub.load_replay("a", Some(&first.to_string()));
        assert_eq!(replay.close_reason, None);
        assert_eq!(replay.entries.len(), 1);
        assert_eq!(replay.entries[0].seq, second);
        assert_eq!(replay.cursor, second);
        let fresh = hub.load_replay("a", None);
        assert!(fresh.entries.is_empty());
        assert_eq!(fresh.cursor, second);
    }

    #[test]
    fn a_lagged_receiver_sees_the_lag() {
        let clock = Arc::new(TestClock::new(10));
        let cfg = HubConfig {
            channel_capacity: 2,
            ..HubConfig::default()
        };
        let hub = hub(&clock, cfg);
        let (socket, _sink) = {
            struct Sink;
            impl crate::delivery::SocketSink for Sink {
                fn send(
                    &self,
                    _frame: &str,
                ) -> Result<crate::delivery::SendStatus, crate::BoxError> {
                    Ok(crate::delivery::SendStatus::Sent)
                }
                fn end(&self, _code: u16, _reason: &str) {}
            }
            let sink = Arc::new(Sink);
            (
                MemberSocket::new(
                    crate::delivery::MemberSocketData {
                        workspace_id: "a".to_owned(),
                        user_id: "u".to_owned(),
                        chat_scope: crate::token::ChatScope::All,
                        team_ids: Vec::new(),
                        iat_ms: 0,
                    },
                    sink.clone(),
                ),
                sink,
            )
        };
        let mut attached = hub.attach_member(&socket, None);
        for _ in 0..3 {
            hub.append_workspace_events("a", typing());
        }
        assert!(matches!(
            attached.rx.try_recv(),
            Err(broadcast::error::TryRecvError::Lagged(1))
        ));
    }

    #[tokio::test(start_paused = true)]
    async fn maintenance_runs_only_while_a_buffer_is_non_empty() {
        let clock = Arc::new(TestClock::new(1_000));
        let hub = hub(&clock, HubConfig::default());
        assert!(!hub.maintenance_running());
        hub.append_workspace_events("a", typing());
        assert!(hub.maintenance_running());
        clock.advance(MAX_BUFFER_AGE_MS + 1);
        tokio::time::sleep(Duration::from_millis(MAINTENANCE_INTERVAL_MS + 1)).await;
        assert!(!hub.maintenance_running());
        assert_eq!(hub.bus_count(), 0);
    }
}

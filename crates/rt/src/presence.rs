//! Presence reporting (spec 2.6, 3.6; `apps/realtime/src/lib/presence-*`).
//!
//! The gateway reports the connected user ids of each workspace to a
//! [`PresenceSink`] in process (decision D3: no HTTP hop, no token, no
//! `bodyHash`). The reporter coalesces, sweeps stalest first, runs at most
//! `concurrency` reports at once and never reports one workspace twice at
//! once.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use chatbotx_kv::{BoxFuture, Clock, Kv, SystemClock};

use crate::BoxError;
use crate::protocol::{PRESENCE_TTL_MS, truncate_presence_user_ids, workspace_presence_key};

/// The in-process presence report target (`workspacePresenceService`).
pub trait PresenceSink: Send + Sync {
    /// Renews the presence leases of `user_ids` (at most 5 000) in
    /// `workspace_id`.
    fn report<'a>(
        &'a self,
        workspace_id: &'a str,
        user_ids: &'a [String],
    ) -> BoxFuture<'a, Result<(), BoxError>>;
}

/// A sink that drops every report.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoPresence;

impl PresenceSink for NoPresence {
    fn report<'a>(
        &'a self,
        _workspace_id: &'a str,
        _user_ids: &'a [String],
    ) -> BoxFuture<'a, Result<(), BoxError>> {
        Box::pin(async { Ok(()) })
    }
}

/// The lease part of `workspacePresenceService.heartbeatMany`:
/// `kv::Presence::heartbeat_many("workspace:presence:<ws>", ids, 20 000)`.
/// Phase 3 adds `markOnlineBulk` for the newly live ids.
pub struct KvPresence {
    kv: Arc<Kv>,
}

impl KvPresence {
    pub fn new(kv: Arc<Kv>) -> KvPresence {
        KvPresence { kv }
    }
}

impl PresenceSink for KvPresence {
    fn report<'a>(
        &'a self,
        workspace_id: &'a str,
        user_ids: &'a [String],
    ) -> BoxFuture<'a, Result<(), BoxError>> {
        let members: Vec<&str> = user_ids.iter().map(String::as_str).collect();
        self.kv.presence.heartbeat_many(
            &workspace_presence_key(workspace_id),
            &members,
            PRESENCE_TTL_MS,
        );
        Box::pin(async { Ok(()) })
    }
}

/// `reportWorkspacePresence`: truncates to 5 000 ids, calls the sink;
/// `true` once accepted, `false` (error logged) when it failed. Never
/// panics, never retries.
pub async fn report_workspace_presence(
    sink: &dyn PresenceSink,
    workspace_id: &str,
    user_ids: &[String],
) -> bool {
    let bounded = truncate_presence_user_ids(user_ids);
    match sink.report(workspace_id, &bounded).await {
        Ok(()) => true,
        Err(err) => {
            tracing::error!(
                err = %err,
                workspace_id = %workspace_id,
                "Failed to report workspace presence"
            );
            false
        }
    }
}

/// One report: resolves `Ok(true)` when the lease was renewed, `Ok(false)`
/// when the report failed, `Err` on an unexpected error.
pub type ReportFn =
    Arc<dyn Fn(String, Vec<String>) -> BoxFuture<'static, Result<bool, BoxError>> + Send + Sync>;

/// `getConnectedWorkspaceIds`.
pub type WorkspaceIdsFn = Arc<dyn Fn() -> Vec<String> + Send + Sync>;
/// `getUserIds(workspaceId)`.
pub type UserIdsFn = Arc<dyn Fn(&str) -> Vec<String> + Send + Sync>;
/// `now()` in ms.
pub type NowFn = Arc<dyn Fn() -> i64 + Send + Sync>;
/// `onError(error, workspaceId)`.
pub type OnErrorFn = Arc<dyn Fn(&BoxError, &str) + Send + Sync>;

/// The options of [`create_presence_reporter`] (`PresenceReporterOptions`).
#[derive(Clone)]
pub struct PresenceReporterOptions {
    pub concurrency: usize,
    pub get_connected_workspace_ids: WorkspaceIdsFn,
    pub get_user_ids: UserIdsFn,
    /// Default: the system clock in ms.
    pub now: Option<NowFn>,
    pub on_error: Option<OnErrorFn>,
    pub report: ReportFn,
}

/// An insertion-ordered set (the JS `Set` the TS queues use).
#[derive(Default)]
struct OrderedSet {
    next: u64,
    order: BTreeMap<u64, String>,
    index: HashMap<String, u64>,
}

impl OrderedSet {
    fn contains(&self, key: &str) -> bool {
        self.index.contains_key(key)
    }

    /// Adds at the end; keeps the position of a present key.
    fn insert(&mut self, key: &str) {
        if self.index.contains_key(key) {
            return;
        }
        self.next += 1;
        self.order.insert(self.next, key.to_owned());
        self.index.insert(key.to_owned(), self.next);
    }

    fn remove(&mut self, key: &str) {
        if let Some(position) = self.index.remove(key) {
            self.order.remove(&position);
        }
    }

    fn pop_front(&mut self) -> Option<String> {
        let (_, key) = self.order.pop_first()?;
        self.index.remove(&key);
        Some(key)
    }

    fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    fn clear(&mut self) {
        self.order.clear();
        self.index.clear();
    }
}

#[derive(Default)]
struct ReporterState {
    priority_queue: OrderedSet,
    sweep_queue: OrderedSet,
    in_flight: HashSet<String>,
    rerun_after_flight: HashSet<String>,
    last_reported_at: HashMap<String, i64>,
    active_reports: usize,
    stopped: bool,
}

impl ReporterState {
    fn take_next(&mut self) -> Option<String> {
        if self.priority_queue.is_empty() {
            self.sweep_queue.pop_front()
        } else {
            self.priority_queue.pop_front()
        }
    }

    fn enqueue(&mut self, workspace_id: &str, is_priority: bool) {
        if self.stopped {
            return;
        }
        if self.in_flight.contains(workspace_id) {
            // The running report may predate this membership change.
            if is_priority {
                self.rerun_after_flight.insert(workspace_id.to_owned());
            }
            return;
        }
        if is_priority {
            self.sweep_queue.remove(workspace_id);
            self.priority_queue.insert(workspace_id);
            return;
        }
        if !self.priority_queue.contains(workspace_id) {
            self.sweep_queue.insert(workspace_id);
        }
    }
}

/// `createPresenceReporter`. Reports run as tokio tasks; `report` is
/// called synchronously when a slot frees (as the TS `pump`).
#[derive(Clone)]
pub struct PresenceReporter {
    inner: Arc<ReporterInner>,
}

struct ReporterInner {
    opts: PresenceReporterOptions,
    state: Mutex<ReporterState>,
}

/// `createPresenceReporter(options)`. Must run inside a tokio runtime.
pub fn create_presence_reporter(opts: PresenceReporterOptions) -> PresenceReporter {
    PresenceReporter {
        inner: Arc::new(ReporterInner {
            opts,
            state: Mutex::new(ReporterState::default()),
        }),
    }
}

impl ReporterInner {
    fn state(&self) -> MutexGuard<'_, ReporterState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn now(&self) -> i64 {
        match &self.opts.now {
            Some(now) => now(),
            None => SystemClock.now_ms(),
        }
    }

    /// Starts reports while slots are free. A loop, never recursion: an
    /// empty workspace is skipped in place.
    fn pump(self: &Arc<Self>) {
        loop {
            let (workspace_id, user_ids) = {
                let mut state = self.state();
                if state.stopped || state.active_reports >= self.opts.concurrency {
                    return;
                }
                let Some(workspace_id) = state.take_next() else {
                    return;
                };
                let user_ids = (self.opts.get_user_ids)(&workspace_id);
                if user_ids.is_empty() {
                    state.last_reported_at.remove(&workspace_id);
                    continue;
                }
                state.active_reports += 1;
                state.in_flight.insert(workspace_id.clone());
                (workspace_id, user_ids)
            };
            let report = (self.opts.report)(workspace_id.clone(), user_ids);
            let inner = Arc::clone(self);
            tokio::spawn(async move {
                let result = report.await;
                inner.finish(&workspace_id, result);
            });
        }
    }

    fn finish(self: &Arc<Self>, workspace_id: &str, result: Result<bool, BoxError>) {
        // Only a renewed lease counts as fresh: a failed report leaves the
        // workspace among the stalest so the next sweep retries it first.
        let fresh_at = match &result {
            Ok(true) => Some(self.now()),
            Ok(false) => None,
            Err(err) => {
                if let Some(on_error) = &self.opts.on_error {
                    on_error(err, workspace_id);
                }
                None
            }
        };
        {
            let mut state = self.state();
            if let Some(at) = fresh_at {
                state.last_reported_at.insert(workspace_id.to_owned(), at);
            }
            state.in_flight.remove(workspace_id);
            state.active_reports = state.active_reports.saturating_sub(1);
            if state.rerun_after_flight.remove(workspace_id) {
                state.enqueue(workspace_id, true);
            }
        }
        self.pump();
    }
}

impl PresenceReporter {
    /// Queues a membership change ahead of any pending periodic reports.
    pub fn report_now(&self, workspace_id: &str) {
        self.inner.state().enqueue(workspace_id, true);
        self.inner.pump();
    }

    /// Queues every connected workspace, longest-unreported first.
    pub fn sweep(&self) {
        let connected = (self.inner.opts.get_connected_workspace_ids)();
        {
            let mut state = self.inner.state();
            // Re-queue from scratch so a backlog left by the previous sweep
            // is re-ordered stalest-first too.
            state.sweep_queue.clear();
            let connected_set: HashSet<&str> = connected.iter().map(String::as_str).collect();
            state
                .last_reported_at
                .retain(|workspace_id, _| connected_set.contains(workspace_id.as_str()));
            let mut stalest_first = connected.clone();
            stalest_first.sort_by_key(|workspace_id| {
                state
                    .last_reported_at
                    .get(workspace_id)
                    .copied()
                    .unwrap_or(0)
            });
            for workspace_id in &stalest_first {
                state.enqueue(workspace_id, false);
            }
        }
        self.inner.pump();
    }

    /// Stops starting new reports and clears the queues.
    pub fn stop(&self) {
        let mut state = self.inner.state();
        state.stopped = true;
        state.priority_queue.clear();
        state.sweep_queue.clear();
        state.rerun_after_flight.clear();
        state.last_reported_at.clear();
    }
}

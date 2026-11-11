//! Test support for the ported apps/worker sequence-scheduler tests
//! (producer, consumer, reconcile, retry, revert). Complete fakes; the
//! crate code under test is not.

#![allow(dead_code)]

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use chatbotx_db::Db;
use chatbotx_jobs::sequence::consumer::{
    AuditActor, DispatchRow, ScheduleConnector, SequenceConsumerServices, StepRow, StepValidation,
    current_audit_actor,
};
use chatbotx_jobs::sequence::producer::{
    PendingDispatchLookup, PendingDispatchRef, ProducerBackend,
};
use chatbotx_jobs::sequence::reconcile::{PendingDue, PendingDueQuery, ReconcileRepo};
use chatbotx_jobs::sequence::schedule::{
    DispatchLock, DispatchSchedule, ScheduleEntry, ScheduleError, ScheduleKind,
};
use chatbotx_jobs::{
    AddResult, BoxError, BoxFuture, Enqueuer, JobOptions, JobsError, NewJob, QueueName,
};
use serde_json::Value;
use tokio::sync::{Notify, Semaphore};

use crate::support::{EnqueueCall, FakeEnqueuer};

// ---------------------------------------------------------------------------
// FakeDispatchSchedule (replaces the vitest `SchedulerClient` mock)
// ---------------------------------------------------------------------------

/// One recorded schedule call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScheduleCall {
    AddToSchedule {
        bucket: u32,
        id: String,
        at_ms: i64,
    },
    RemoveFromSchedule {
        bucket: u32,
        id: String,
    },
    AddToRetry {
        bucket: u32,
        id: String,
        at_ms: i64,
    },
    RemoveFromRetry {
        bucket: u32,
        id: String,
    },
    RemoveFromAll {
        bucket: u32,
        id: String,
    },
    GetDue {
        kind: ScheduleKind,
        bucket: u32,
        now_ms: i64,
        limit: u32,
    },
    Count {
        kind: ScheduleKind,
        bucket: u32,
    },
    BatchAdd(Vec<ScheduleEntry>),
    Members {
        kind: ScheduleKind,
        bucket: u32,
    },
    TryLock {
        bucket: u32,
        id: String,
        ttl_secs: u64,
    },
}

/// Scripted `DispatchSchedule`: `get_due` / `members` answers are queued per
/// set (each answer is used once, then `[]`), counts are fixed per set,
/// locks are real (a held key fails with `LockAcquisition`), and any
/// operation can be made to fail.
#[derive(Default)]
pub struct FakeDispatchSchedule {
    calls: Mutex<Vec<ScheduleCall>>,
    due: Mutex<BTreeMap<ScheduleKind, VecDeque<Vec<String>>>>,
    members: Mutex<BTreeMap<ScheduleKind, VecDeque<Vec<String>>>>,
    counts: Mutex<BTreeMap<ScheduleKind, u64>>,
    held: Arc<Mutex<HashSet<String>>>,
    lock_fail_for: Mutex<HashMap<String, ScheduleError>>,
    lock_fail_once: Mutex<VecDeque<ScheduleError>>,
    fail_next: Mutex<HashMap<&'static str, VecDeque<ScheduleError>>>,
}

/// The Redis lock key format (`SchedulerClient.getLockKey`).
pub fn lock_key(bucket: u32, dispatch_id: &str) -> String {
    format!("seq:dispatch:{{{bucket}}}:lock:{dispatch_id}")
}

impl FakeDispatchSchedule {
    pub fn new() -> Arc<FakeDispatchSchedule> {
        Arc::new(FakeDispatchSchedule::default())
    }

    pub fn push_due(&self, kind: ScheduleKind, ids: &[&str]) {
        self.due
            .lock()
            .unwrap()
            .entry(kind)
            .or_default()
            .push_back(ids.iter().map(|s| s.to_string()).collect());
    }

    pub fn push_members(&self, kind: ScheduleKind, ids: &[&str]) {
        self.members
            .lock()
            .unwrap()
            .entry(kind)
            .or_default()
            .push_back(ids.iter().map(|s| s.to_string()).collect());
    }

    pub fn set_count(&self, kind: ScheduleKind, count: u64) {
        self.counts.lock().unwrap().insert(kind, count);
    }

    /// `try_lock` for this dispatch id always fails with `error`.
    pub fn fail_lock_for(&self, dispatch_id: &str, error: ScheduleError) {
        self.lock_fail_for
            .lock()
            .unwrap()
            .insert(dispatch_id.to_owned(), error);
    }

    /// The next `try_lock` (any id) fails with `error`.
    pub fn fail_lock_once(&self, error: ScheduleError) {
        self.lock_fail_once.lock().unwrap().push_back(error);
    }

    /// The next call of `op` (method name, e.g. `"batch_add_to_schedule"`)
    /// fails with `error`.
    pub fn fail_next(&self, op: &'static str, error: ScheduleError) {
        self.fail_next
            .lock()
            .unwrap()
            .entry(op)
            .or_default()
            .push_back(error);
    }

    pub fn calls(&self) -> Vec<ScheduleCall> {
        self.calls.lock().unwrap().clone()
    }

    pub fn clear_calls(&self) {
        self.calls.lock().unwrap().clear();
    }

    pub fn get_due_calls(&self) -> usize {
        self.calls()
            .iter()
            .filter(|c| matches!(c, ScheduleCall::GetDue { .. }))
            .count()
    }

    pub fn try_lock_calls(&self) -> Vec<(u32, String, u64)> {
        self.calls()
            .into_iter()
            .filter_map(|c| match c {
                ScheduleCall::TryLock {
                    bucket,
                    id,
                    ttl_secs,
                } => Some((bucket, id, ttl_secs)),
                _ => None,
            })
            .collect()
    }

    pub fn count_calls(&self, kind: ScheduleKind) -> usize {
        self.calls()
            .iter()
            .filter(|c| matches!(c, ScheduleCall::Count { kind: k, .. } if *k == kind))
            .count()
    }

    pub fn has(&self, call: &ScheduleCall) -> bool {
        self.calls().contains(call)
    }

    fn record(&self, call: ScheduleCall) {
        self.calls.lock().unwrap().push(call);
    }

    fn take_failure(&self, op: &'static str) -> Result<(), ScheduleError> {
        match self
            .fail_next
            .lock()
            .unwrap()
            .get_mut(op)
            .and_then(VecDeque::pop_front)
        {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }
}

impl DispatchSchedule for FakeDispatchSchedule {
    fn add_to_schedule<'a>(
        &'a self,
        bucket: u32,
        dispatch_id: &'a str,
        at_ms: i64,
    ) -> BoxFuture<'a, Result<(), ScheduleError>> {
        self.record(ScheduleCall::AddToSchedule {
            bucket,
            id: dispatch_id.to_owned(),
            at_ms,
        });
        let r = self.take_failure("add_to_schedule");
        Box::pin(async move { r })
    }

    fn remove_from_schedule<'a>(
        &'a self,
        bucket: u32,
        dispatch_id: &'a str,
    ) -> BoxFuture<'a, Result<(), ScheduleError>> {
        self.record(ScheduleCall::RemoveFromSchedule {
            bucket,
            id: dispatch_id.to_owned(),
        });
        let r = self.take_failure("remove_from_schedule");
        Box::pin(async move { r })
    }

    fn add_to_retry<'a>(
        &'a self,
        bucket: u32,
        dispatch_id: &'a str,
        at_ms: i64,
    ) -> BoxFuture<'a, Result<(), ScheduleError>> {
        self.record(ScheduleCall::AddToRetry {
            bucket,
            id: dispatch_id.to_owned(),
            at_ms,
        });
        let r = self.take_failure("add_to_retry");
        Box::pin(async move { r })
    }

    fn remove_from_retry<'a>(
        &'a self,
        bucket: u32,
        dispatch_id: &'a str,
    ) -> BoxFuture<'a, Result<(), ScheduleError>> {
        self.record(ScheduleCall::RemoveFromRetry {
            bucket,
            id: dispatch_id.to_owned(),
        });
        let r = self.take_failure("remove_from_retry");
        Box::pin(async move { r })
    }

    fn remove_from_all<'a>(
        &'a self,
        bucket: u32,
        dispatch_id: &'a str,
    ) -> BoxFuture<'a, Result<(), ScheduleError>> {
        self.record(ScheduleCall::RemoveFromAll {
            bucket,
            id: dispatch_id.to_owned(),
        });
        let r = self.take_failure("remove_from_all");
        Box::pin(async move { r })
    }

    fn get_due(
        &self,
        kind: ScheduleKind,
        bucket: u32,
        now_ms: i64,
        limit: u32,
    ) -> BoxFuture<'_, Result<Vec<String>, ScheduleError>> {
        self.record(ScheduleCall::GetDue {
            kind,
            bucket,
            now_ms,
            limit,
        });
        let r = self.take_failure("get_due").map(|()| {
            self.due
                .lock()
                .unwrap()
                .get_mut(&kind)
                .and_then(VecDeque::pop_front)
                .unwrap_or_default()
        });
        Box::pin(async move { r })
    }

    fn count(&self, kind: ScheduleKind, bucket: u32) -> BoxFuture<'_, Result<u64, ScheduleError>> {
        self.record(ScheduleCall::Count { kind, bucket });
        let r = self
            .take_failure("count")
            .map(|()| self.counts.lock().unwrap().get(&kind).copied().unwrap_or(0));
        Box::pin(async move { r })
    }

    fn batch_add_to_schedule(
        &self,
        entries: Vec<ScheduleEntry>,
    ) -> BoxFuture<'_, Result<(), ScheduleError>> {
        self.record(ScheduleCall::BatchAdd(entries));
        let r = self.take_failure("batch_add_to_schedule");
        Box::pin(async move { r })
    }

    fn members(
        &self,
        kind: ScheduleKind,
        bucket: u32,
    ) -> BoxFuture<'_, Result<Vec<String>, ScheduleError>> {
        self.record(ScheduleCall::Members { kind, bucket });
        let r = self.take_failure("members").map(|()| {
            self.members
                .lock()
                .unwrap()
                .get_mut(&kind)
                .and_then(VecDeque::pop_front)
                .unwrap_or_default()
        });
        Box::pin(async move { r })
    }

    fn try_lock<'a>(
        &'a self,
        bucket: u32,
        dispatch_id: &'a str,
        ttl_secs: u64,
    ) -> BoxFuture<'a, Result<DispatchLock, ScheduleError>> {
        self.record(ScheduleCall::TryLock {
            bucket,
            id: dispatch_id.to_owned(),
            ttl_secs,
        });
        let key = lock_key(bucket, dispatch_id);
        let result = if let Some(e) = self.lock_fail_once.lock().unwrap().pop_front() {
            Err(e)
        } else if let Some(e) = self.lock_fail_for.lock().unwrap().get(dispatch_id).cloned() {
            Err(e)
        } else if !self.held.lock().unwrap().insert(key.clone()) {
            Err(ScheduleError::LockAcquisition { key: key.clone() })
        } else {
            let held = Arc::clone(&self.held);
            let released = key.clone();
            Ok(DispatchLock::new(
                key,
                Box::new(move || {
                    held.lock().unwrap().remove(&released);
                }),
            ))
        };
        Box::pin(async move { result })
    }
}

// ---------------------------------------------------------------------------
// Producer collaborators
// ---------------------------------------------------------------------------

/// `ProducerBackend` + `ScheduleConnector` that counts its calls.
pub struct FakeBackend {
    pub schedule: Arc<FakeDispatchSchedule>,
    pub queue: Option<Arc<dyn Enqueuer>>,
    pub connects: AtomicUsize,
    pub queue_lookups: AtomicUsize,
}

impl FakeBackend {
    pub fn new(
        schedule: Arc<FakeDispatchSchedule>,
        queue: Option<Arc<dyn Enqueuer>>,
    ) -> Arc<FakeBackend> {
        Arc::new(FakeBackend {
            schedule,
            queue,
            connects: AtomicUsize::new(0),
            queue_lookups: AtomicUsize::new(0),
        })
    }

    pub fn connects(&self) -> usize {
        self.connects.load(Ordering::SeqCst)
    }

    pub fn queue_lookups(&self) -> usize {
        self.queue_lookups.load(Ordering::SeqCst)
    }
}

impl ProducerBackend for FakeBackend {
    fn connect_schedule(&self) -> BoxFuture<'_, Result<Arc<dyn DispatchSchedule>, JobsError>> {
        self.connects.fetch_add(1, Ordering::SeqCst);
        let schedule: Arc<dyn DispatchSchedule> = self.schedule.clone();
        Box::pin(async move { Ok(schedule) })
    }

    fn sequence_queue(&self) -> BoxFuture<'_, Result<Option<Arc<dyn Enqueuer>>, JobsError>> {
        self.queue_lookups.fetch_add(1, Ordering::SeqCst);
        let queue = self.queue.clone();
        Box::pin(async move { Ok(queue) })
    }
}

impl ScheduleConnector for FakeBackend {
    fn connect_schedule(&self) -> BoxFuture<'_, Result<Arc<dyn DispatchSchedule>, JobsError>> {
        ProducerBackend::connect_schedule(self)
    }
}

/// `listPendingWorkspaceIds`: returns the configured rows whose id was asked for.
#[derive(Default)]
pub struct FakePendingLookup {
    rows: Mutex<Vec<PendingDispatchRef>>,
    pub asked: Mutex<Vec<Vec<String>>>,
}

impl FakePendingLookup {
    pub fn with(rows: &[(&str, &str)]) -> Arc<FakePendingLookup> {
        Arc::new(FakePendingLookup {
            rows: Mutex::new(
                rows.iter()
                    .map(|(id, ws)| PendingDispatchRef {
                        id: id.to_string(),
                        workspace_id: ws.to_string(),
                    })
                    .collect(),
            ),
            asked: Mutex::new(Vec::new()),
        })
    }
}

impl PendingDispatchLookup for FakePendingLookup {
    fn list_pending_workspace_ids<'a>(
        &'a self,
        dispatch_ids: &'a [String],
    ) -> BoxFuture<'a, Result<Vec<PendingDispatchRef>, JobsError>> {
        self.asked.lock().unwrap().push(dispatch_ids.to_vec());
        let rows = self
            .rows
            .lock()
            .unwrap()
            .iter()
            .filter(|r| dispatch_ids.contains(&r.id))
            .cloned()
            .collect();
        Box::pin(async move { Ok(rows) })
    }
}

/// A `FakeEnqueuer` whose `add_bulk` records the call, signals `started`,
/// then waits until `release()` (an unresolved `addBulk` promise).
pub struct GatedEnqueuer {
    pub inner: Arc<FakeEnqueuer>,
    pub started: Notify,
    gate: Semaphore,
}

impl GatedEnqueuer {
    pub fn new() -> Arc<GatedEnqueuer> {
        Arc::new(GatedEnqueuer {
            inner: FakeEnqueuer::new(),
            started: Notify::new(),
            gate: Semaphore::new(0),
        })
    }

    pub fn release(&self) {
        self.gate.add_permits(1);
    }
}

impl Enqueuer for GatedEnqueuer {
    fn add(
        &self,
        queue: QueueName,
        name: &str,
        data: Value,
        opts: JobOptions,
    ) -> BoxFuture<'_, Result<AddResult, JobsError>> {
        self.inner.add(queue, name, data, opts)
    }

    fn add_bulk(
        &self,
        queue: QueueName,
        jobs: Vec<NewJob>,
    ) -> BoxFuture<'_, Result<Vec<AddResult>, JobsError>> {
        Box::pin(async move {
            let result = self.inner.add_bulk(queue, jobs).await;
            self.started.notify_one();
            let permit = self.gate.acquire().await.expect("gate open");
            permit.forget();
            result
        })
    }
}

// ---------------------------------------------------------------------------
// Consumer collaborators
// ---------------------------------------------------------------------------

/// The mocked services of `sequence-worker-consumer.test.ts`: every check
/// passes; `fetch_dispatch` answers the configured result.
pub struct FakeConsumerServices {
    pub dispatch: Mutex<Option<DispatchRow>>,
    /// When set, `fetch_dispatch` fails with this lock error once.
    pub fetch_error: Mutex<Option<(String, String)>>,
    pub fetches: AtomicUsize,
}

impl FakeConsumerServices {
    pub fn new(dispatch: Option<DispatchRow>) -> Arc<FakeConsumerServices> {
        Arc::new(FakeConsumerServices {
            dispatch: Mutex::new(dispatch),
            fetch_error: Mutex::new(None),
            fetches: AtomicUsize::new(0),
        })
    }

    pub fn fetches(&self) -> usize {
        self.fetches.load(Ordering::SeqCst)
    }
}

fn ok<T: Send + 'static>(value: T) -> BoxFuture<'static, Result<T, BoxError>> {
    Box::pin(async move { Ok(value) })
}

impl SequenceConsumerServices for FakeConsumerServices {
    fn is_blocked_workspace<'a>(
        &'a self,
        _workspace_id: &'a str,
    ) -> BoxFuture<'a, Result<bool, BoxError>> {
        ok(false)
    }

    fn fetch_dispatch<'a>(
        &'a self,
        _dispatch_id: &'a str,
        _workspace_id: &'a str,
    ) -> BoxFuture<'a, Result<Option<DispatchRow>, BoxError>> {
        self.fetches.fetch_add(1, Ordering::SeqCst);
        if let Some((key, message)) = self.fetch_error.lock().unwrap().take() {
            let err: BoxError =
                Box::new(chatbotx_jobs::sequence::consumer::LockAcquisitionError { key, message });
            return Box::pin(async move { Err(err) });
        }
        ok(self.dispatch.lock().unwrap().clone())
    }

    fn validate_dispatch<'a>(
        &'a self,
        _dispatch: &'a DispatchRow,
    ) -> BoxFuture<'a, Result<bool, BoxError>> {
        ok(true)
    }

    fn is_dispatch_ready(&self, _dispatch: &DispatchRow, _now_ms: i64) -> bool {
        true
    }

    fn lock_dispatch<'a>(
        &'a self,
        _dispatch: &'a DispatchRow,
    ) -> BoxFuture<'a, Result<bool, BoxError>> {
        ok(true)
    }

    fn fetch_step<'a>(
        &'a self,
        _step_id: &'a str,
    ) -> BoxFuture<'a, Result<Option<StepRow>, BoxError>> {
        ok(Some(StepRow {
            id: "step-1".to_owned(),
            sequence_id: "sequence-1".to_owned(),
        }))
    }

    fn validate_step<'a>(
        &'a self,
        _step: &'a StepRow,
    ) -> BoxFuture<'a, Result<StepValidation, BoxError>> {
        ok(StepValidation {
            valid: true,
            reason: None,
        })
    }

    fn mark_dispatch_canceled<'a>(
        &'a self,
        _dispatch_id: &'a str,
        _workspace_id: &'a str,
        _reason: &'a str,
    ) -> BoxFuture<'a, Result<(), BoxError>> {
        ok(())
    }

    fn revert_dispatch_to_pending<'a>(
        &'a self,
        _dispatch_id: &'a str,
        _workspace_id: &'a str,
    ) -> BoxFuture<'a, Result<(), BoxError>> {
        ok(())
    }
}

/// An `Enqueuer` that records each add together with the audit actor of
/// the task at that moment (`getAuditActor()` inside `integrationQueue.add`).
#[derive(Default)]
pub struct AuditEnqueuer {
    pub calls: Mutex<Vec<(EnqueueCall, Option<AuditActor>)>>,
}

impl Enqueuer for AuditEnqueuer {
    fn add(
        &self,
        queue: QueueName,
        name: &str,
        data: Value,
        opts: JobOptions,
    ) -> BoxFuture<'_, Result<AddResult, JobsError>> {
        let actor = current_audit_actor();
        let job_id = opts.job_id.clone().unwrap_or_default();
        self.calls.lock().unwrap().push((
            EnqueueCall {
                queue,
                name: name.to_owned(),
                data,
                opts,
            },
            actor,
        ));
        Box::pin(async move {
            Ok(AddResult {
                job_id,
                created: true,
            })
        })
    }

    fn add_bulk(
        &self,
        _queue: QueueName,
        _jobs: Vec<NewJob>,
    ) -> BoxFuture<'_, Result<Vec<AddResult>, JobsError>> {
        Box::pin(async move { Err(JobsError::Other("add_bulk not expected".to_owned())) })
    }
}

// ---------------------------------------------------------------------------
// Reconcile collaborators
// ---------------------------------------------------------------------------

/// `sequenceDispatchModel.findMany` for the reconcile job: the first scan
/// answers the configured rows, later scans `[]`.
#[derive(Default)]
pub struct FakeReconcileRepo {
    pub due: Mutex<VecDeque<Vec<PendingDue>>>,
    pub pending_ids: Mutex<Vec<String>>,
    pub queries: Mutex<Vec<PendingDueQuery>>,
    pub id_lookups: Mutex<Vec<Vec<String>>>,
}

impl ReconcileRepo for FakeReconcileRepo {
    fn find_pending_due(
        &self,
        query: PendingDueQuery,
    ) -> BoxFuture<'_, Result<Vec<PendingDue>, JobsError>> {
        self.queries.lock().unwrap().push(query);
        let rows = self.due.lock().unwrap().pop_front().unwrap_or_default();
        Box::pin(async move { Ok(rows) })
    }

    fn find_pending_ids<'a>(
        &'a self,
        ids: &'a [String],
    ) -> BoxFuture<'a, Result<Vec<String>, JobsError>> {
        self.id_lookups.lock().unwrap().push(ids.to_vec());
        let pending = self.pending_ids.lock().unwrap().clone();
        let found = ids
            .iter()
            .filter(|i| pending.contains(i))
            .cloned()
            .collect();
        Box::pin(async move { Ok(found) })
    }
}

// ---------------------------------------------------------------------------
// SequenceDispatch rows (P2.4 DDL, `crates/db/migrations/0002_messaging.sql`)
// ---------------------------------------------------------------------------

/// Fixed integer ids for the TypeScript string ids of the DB-backed tests
/// (P2.4: every id is INTEGER; the API takes and returns decimal strings,
/// DECISIONS P2.2). One mapping, used by every seed and assertion:
///
/// | TS id                              | const   | value |
/// |------------------------------------|---------|-------|
/// | `"d1"`                             | `D1`    | 701   |
/// | `"d2"`                             | `D2`    | 702   |
/// | `"d3"`                             | `D3`    | 703   |
/// | `"d4"` (Rust-only keep row)        | `D4`    | 704   |
/// | `"d5"` (Rust-only keep row)        | `D5`    | 705   |
/// | `"ws1"` / `"w1"` / `"workspace-1"` | `WS1`   | 101   |
/// | `"ws2"` / `"w2"`                   | `WS2`   | 102   |
///
/// Parent rows (not in the TS tests; needed by the foreign keys), one set
/// per workspace `ws`: `User` 900 (shared), `Contact` ws+100,
/// `Inbox` ws+200, `ContactInbox` ws+300, `Sequence` ws+400,
/// `SequenceStep` ws+500, `ContactOnSequence` (the enrollment) ws+700.
pub mod ids {
    pub const D1: i64 = 701;
    pub const D2: i64 = 702;
    pub const D3: i64 = 703;
    pub const D4: i64 = 704;
    pub const D5: i64 = 705;
    pub const WS1: i64 = 101;
    pub const WS2: i64 = 102;
    pub const USER: i64 = 900;

    /// The decimal string the API takes and returns for an id.
    pub fn s(id: i64) -> String {
        id.to_string()
    }
}

/// A `SequenceDispatch` row to seed (column names of the P2.4 DDL; times
/// are unix ms).
#[derive(Debug, Clone)]
pub struct SeedDispatch {
    pub id: i64,
    pub workspace_id: i64,
    pub status: &'static str,
    pub bucket: u32,
    pub run_at_ms: i64,
    pub locked_at: Option<i64>,
    pub lock_owner: Option<&'static str>,
    pub last_error: Option<&'static str>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl SeedDispatch {
    pub fn new(id: i64, workspace_id: i64, status: &'static str) -> SeedDispatch {
        SeedDispatch {
            id,
            workspace_id,
            status,
            bucket: 0,
            run_at_ms: 1_700_000_000_000,
            locked_at: None,
            lock_owner: None,
            last_error: None,
            created_at: 1_700_000_000_000,
            updated_at: 1_700_000_000_000,
        }
    }
}

/// `INSERT OR IGNORE` of the parent rows of one workspace's dispatches:
/// User → Workspace → Contact / Inbox → ContactInbox, Sequence →
/// SequenceStep, and the ContactOnSequence enrollment (composite FK).
fn ensure_parents(
    tx: &chatbotx_db::rusqlite::Connection,
    ws: i64,
) -> chatbotx_db::rusqlite::Result<()> {
    use chatbotx_db::rusqlite::params;
    tx.execute(
        r#"INSERT OR IGNORE INTO "User" ("id", "email") VALUES (?1, 'owner@example.com')"#,
        params![ids::USER],
    )?;
    tx.execute(
        r#"INSERT OR IGNORE INTO "Workspace" ("id", "name", "ownerId") VALUES (?1, ?2, ?3)"#,
        params![ws, format!("workspace {ws}"), ids::USER],
    )?;
    tx.execute(
        r#"INSERT OR IGNORE INTO "Contact" ("id", "workspaceId") VALUES (?1, ?2)"#,
        params![ws + 100, ws],
    )?;
    tx.execute(
        r#"INSERT OR IGNORE INTO "Inbox" ("id", "name", "channel", "sourceId", "workspaceId")
           VALUES (?1, 'inbox', 'webchat', ?2, ?3)"#,
        params![ws + 200, format!("source-{ws}"), ws],
    )?;
    tx.execute(
        r#"INSERT OR IGNORE INTO "ContactInbox"
           ("id", "originalContactId", "contactId", "inboxId", "channel", "source", "sourceId")
           VALUES (?1, ?2, ?2, ?3, 'webchat', 'webchat', ?4)"#,
        params![ws + 300, ws + 100, ws + 200, format!("contact-source-{ws}")],
    )?;
    tx.execute(
        r#"INSERT OR IGNORE INTO "Sequence" ("id", "name", "workspaceId") VALUES (?1, 'sequence', ?2)"#,
        params![ws + 400, ws],
    )?;
    tx.execute(
        r#"INSERT OR IGNORE INTO "SequenceStep" ("id", "order", "delayDays", "sequenceId")
           VALUES (?1, 0, 0, ?2)"#,
        params![ws + 500, ws + 400],
    )?;
    tx.execute(
        r#"INSERT OR IGNORE INTO "ContactOnSequence" ("id", "contactId", "sequenceId", "workspaceId")
           VALUES (?1, ?2, ?3, ?4)"#,
        params![ws + 700, ws + 100, ws + 400, ws],
    )?;
    Ok(())
}

/// Inserts the rows (and their parents) into the P2.4 tables.
pub async fn seed_dispatches(db: &Db, rows: Vec<SeedDispatch>) {
    db.write(move |tx| {
        for r in &rows {
            ensure_parents(tx, r.workspace_id)?;
            let ws = r.workspace_id;
            tx.execute(
                r#"INSERT INTO "SequenceDispatch" (
                    "id", "workspaceId", "sequenceId", "enrollmentId", "stepId",
                    "contactId", "contactInboxId", "bucket", "runAtMs", "status",
                    "attempt", "idempotencyKey", "lockedAt", "lockOwner", "lastError",
                    "createdAt", "updatedAt"
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 0, ?11, ?12, ?13, ?14, ?15, ?16)"#,
                chatbotx_db::rusqlite::params![
                    r.id,
                    ws,
                    ws + 400,
                    ws + 700,
                    ws + 500,
                    ws + 100,
                    ws + 300,
                    r.bucket,
                    r.run_at_ms,
                    r.status,
                    format!("{ws}:{}:{}:{}", ws + 700, ws + 500, r.id),
                    r.locked_at,
                    r.lock_owner,
                    r.last_error,
                    r.created_at,
                    r.updated_at,
                ],
            )?;
        }
        Ok(())
    })
    .await
    .expect("seed SequenceDispatch rows (needs the P2.4 schema)");
}

/// The columns the tests read back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DispatchSnapshot {
    pub status: String,
    pub locked_at: Option<i64>,
    pub lock_owner: Option<String>,
    pub last_error: Option<String>,
    pub updated_at: i64,
}

pub async fn read_dispatch(db: &Db, id: i64) -> Option<DispatchSnapshot> {
    db.read(move |c| {
        let mut stmt = c.prepare(
            r#"SELECT "status", "lockedAt", "lockOwner", "lastError", "updatedAt"
               FROM "SequenceDispatch" WHERE "id" = ?1"#,
        )?;
        let mut rows = stmt.query([id])?;
        match rows.next()? {
            Some(row) => Ok(Some(DispatchSnapshot {
                status: row.get(0)?,
                locked_at: row.get(1)?,
                lock_owner: row.get(2)?,
                last_error: row.get(3)?,
                updated_at: row.get(4)?,
            })),
            None => Ok(None),
        }
    })
    .await
    .expect("read SequenceDispatch row")
}

/// Every `"SequenceDispatch"` id, sorted.
pub async fn dispatch_ids(db: &Db) -> Vec<i64> {
    db.read(|c| {
        let mut stmt = c.prepare(r#"SELECT "id" FROM "SequenceDispatch" ORDER BY "id""#)?;
        let ids = stmt
            .query_map([], |row| row.get::<_, i64>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ids)
    })
    .await
    .expect("list SequenceDispatch ids")
}

/// An empty env map.
pub fn env(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

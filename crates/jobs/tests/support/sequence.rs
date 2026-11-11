//! Test support for the sequence-scheduler ports (spec 5.2): row seeding on
//! the P2.4 tables, `RecordingSchedule` (a `DispatchSchedule` fake),
//! `TxProbe` (a `TxObserver` that counts transactions and statements),
//! fakes of the injectable dependencies, and the `Harness` that wires them
//! into a `SequenceCtx`.
//!
//! The seeds follow the P2.4 DDL (`0002_messaging.sql`): integer ids from the
//! mapping table below, integer ms times, parent rows created first (foreign
//! keys are ON).

#![allow(dead_code)]

use std::collections::{BTreeMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};

use chatbotx_db::Db;
use chatbotx_db::rusqlite::Connection;
use chatbotx_db::rusqlite::types::Value as SqlValue;
use chatbotx_jobs::BoxFuture;
use chatbotx_jobs::sequence::dispatch::{
    CreateDispatchParams, DispatchInserter, DispatchRecord, NewDispatchRow,
};
use chatbotx_jobs::sequence::next_run::StepTiming;
use chatbotx_jobs::sequence::repo::{
    ContactInboxLookup, ContactInboxRef, DispatchCreator, IdGenerator, NextRunPolicy, SequenceCtx,
    SequenceError, StatementKind, TxObserver,
};
use chatbotx_jobs::sequence::schedule::{
    DispatchLock, DispatchSchedule, ScheduleEntry, ScheduleError, ScheduleKind,
};
use chatbotx_jobs::sequence::send_time::SendWindow;
use serde_json::{Map, Value, json};

use crate::support::TestClock;

/// Shared order log (`order: string[]` of the TypeScript tests).
pub type OrderLog = Arc<Mutex<Vec<String>>>;

pub fn order_log() -> OrderLog {
    Arc::new(Mutex::new(Vec::new()))
}

/// Unix ms of an RFC 3339 instant (`new Date("2024-03-01T12:00:00Z")`).
pub fn iso_ms(text: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(text)
        .unwrap_or_else(|e| panic!("bad instant {text}: {e}"))
        .timestamp_millis()
}

// ---------------------------------------------------------------------------
// TxProbe
// ---------------------------------------------------------------------------

/// Counts `on_begin` / `on_commit` and statements; pushes `"tx-done"` into
/// the order log at each commit (replaces `transactionMock` + `order`).
#[derive(Default)]
pub struct TxProbe {
    begins: Mutex<usize>,
    commits: Mutex<usize>,
    statements: Mutex<Vec<(StatementKind, String)>>,
    order: Mutex<Option<OrderLog>>,
}

impl TxProbe {
    pub fn new(order: &OrderLog) -> Arc<TxProbe> {
        let probe = TxProbe::default();
        *probe.order.lock().unwrap() = Some(Arc::clone(order));
        Arc::new(probe)
    }

    pub fn begins(&self) -> usize {
        *self.begins.lock().unwrap()
    }

    pub fn commits(&self) -> usize {
        *self.commits.lock().unwrap()
    }

    pub fn statements(&self) -> Vec<(StatementKind, String)> {
        self.statements.lock().unwrap().clone()
    }

    /// Statements of `kind` on `table`.
    pub fn count(&self, kind: StatementKind, table: &str) -> usize {
        self.statements()
            .iter()
            .filter(|(k, t)| *k == kind && t == table)
            .count()
    }

    /// Statements of `kind` on any table.
    pub fn count_kind(&self, kind: StatementKind) -> usize {
        self.statements().iter().filter(|(k, _)| *k == kind).count()
    }

    /// Writes (INSERT / UPDATE / DELETE) on any table.
    pub fn writes(&self) -> usize {
        self.statements()
            .iter()
            .filter(|(k, _)| *k != StatementKind::Select)
            .count()
    }
}

impl TxObserver for TxProbe {
    fn on_begin(&self) {
        *self.begins.lock().unwrap() += 1;
    }

    fn on_commit(&self) {
        *self.commits.lock().unwrap() += 1;
        if let Some(order) = self.order.lock().unwrap().as_ref() {
            order.lock().unwrap().push("tx-done".to_owned());
        }
    }

    fn on_statement(&self, kind: StatementKind, table: &str) {
        self.statements
            .lock()
            .unwrap()
            .push((kind, table.to_owned()));
    }
}

// ---------------------------------------------------------------------------
// RecordingSchedule
// ---------------------------------------------------------------------------

/// One recorded schedule call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduleCall {
    /// `add_to_schedule`, `remove_from_schedule`, `add_to_retry`, ...
    pub op: &'static str,
    pub bucket: u32,
    pub dispatch_id: String,
    pub at_ms: Option<i64>,
}

/// In-memory `DispatchSchedule` that records every call (replaces the
/// `SchedulerClient` mock). Calls listed with `fail_call` (1-based, counted
/// over all ops) fail with `ScheduleError::Store(message)`.
/// `add_to_schedule` pushes `"schedule"` into the order log.
#[derive(Default)]
pub struct RecordingSchedule {
    calls: Mutex<Vec<ScheduleCall>>,
    sets: Mutex<BTreeMap<(ScheduleKind, String), (u32, i64)>>,
    failing: Mutex<BTreeMap<usize, String>>,
    locks: Arc<Mutex<HashSet<String>>>,
    order: Mutex<Option<OrderLog>>,
}

impl RecordingSchedule {
    pub fn new(order: &OrderLog) -> Arc<RecordingSchedule> {
        let schedule = RecordingSchedule::default();
        *schedule.order.lock().unwrap() = Some(Arc::clone(order));
        Arc::new(schedule)
    }

    /// The `n`-th call (1-based) fails with `message`.
    pub fn fail_call(&self, n: usize, message: &str) {
        self.failing.lock().unwrap().insert(n, message.to_owned());
    }

    pub fn calls(&self) -> Vec<ScheduleCall> {
        self.calls.lock().unwrap().clone()
    }

    pub fn calls_of(&self, op: &str) -> Vec<ScheduleCall> {
        self.calls().into_iter().filter(|c| c.op == op).collect()
    }

    /// `(bucket, dispatch_id, at_ms)` of every `add_to_schedule` call.
    pub fn scheduled(&self) -> Vec<(u32, String, i64)> {
        self.calls_of("add_to_schedule")
            .into_iter()
            .map(|c| (c.bucket, c.dispatch_id, c.at_ms.unwrap_or_default()))
            .collect()
    }

    /// `(bucket, dispatch_id)` of every `remove_from_schedule` call.
    pub fn removed(&self) -> Vec<(u32, String)> {
        self.calls_of("remove_from_schedule")
            .into_iter()
            .map(|c| (c.bucket, c.dispatch_id))
            .collect()
    }

    fn record(
        &self,
        op: &'static str,
        bucket: u32,
        dispatch_id: &str,
        at_ms: Option<i64>,
    ) -> Result<(), ScheduleError> {
        let n = {
            let mut calls = self.calls.lock().unwrap();
            calls.push(ScheduleCall {
                op,
                bucket,
                dispatch_id: dispatch_id.to_owned(),
                at_ms,
            });
            calls.len()
        };
        if op == "add_to_schedule" {
            if let Some(order) = self.order.lock().unwrap().as_ref() {
                order.lock().unwrap().push("schedule".to_owned());
            }
        }
        match self.failing.lock().unwrap().get(&n) {
            Some(message) => Err(ScheduleError::Store(message.clone())),
            None => Ok(()),
        }
    }

    fn put(&self, kind: ScheduleKind, bucket: u32, dispatch_id: &str, at_ms: i64) {
        self.sets
            .lock()
            .unwrap()
            .insert((kind, dispatch_id.to_owned()), (bucket, at_ms));
    }

    fn take(&self, kind: ScheduleKind, dispatch_id: &str) {
        self.sets
            .lock()
            .unwrap()
            .remove(&(kind, dispatch_id.to_owned()));
    }

    fn ids(&self, kind: ScheduleKind, bucket: u32, max_at: Option<i64>) -> Vec<String> {
        let sets = self.sets.lock().unwrap();
        let mut rows: Vec<(i64, String)> = sets
            .iter()
            .filter(|((k, _), (b, at))| {
                *k == kind && *b == bucket && max_at.is_none_or(|m| *at <= m)
            })
            .map(|((_, id), (_, at))| (*at, id.clone()))
            .collect();
        rows.sort();
        rows.into_iter().map(|(_, id)| id).collect()
    }
}

impl DispatchSchedule for RecordingSchedule {
    fn add_to_schedule<'a>(
        &'a self,
        bucket: u32,
        dispatch_id: &'a str,
        at_ms: i64,
    ) -> BoxFuture<'a, Result<(), ScheduleError>> {
        Box::pin(async move {
            self.record("add_to_schedule", bucket, dispatch_id, Some(at_ms))?;
            self.put(ScheduleKind::Schedule, bucket, dispatch_id, at_ms);
            Ok(())
        })
    }

    fn remove_from_schedule<'a>(
        &'a self,
        bucket: u32,
        dispatch_id: &'a str,
    ) -> BoxFuture<'a, Result<(), ScheduleError>> {
        Box::pin(async move {
            self.record("remove_from_schedule", bucket, dispatch_id, None)?;
            self.take(ScheduleKind::Schedule, dispatch_id);
            Ok(())
        })
    }

    fn add_to_retry<'a>(
        &'a self,
        bucket: u32,
        dispatch_id: &'a str,
        at_ms: i64,
    ) -> BoxFuture<'a, Result<(), ScheduleError>> {
        Box::pin(async move {
            self.record("add_to_retry", bucket, dispatch_id, Some(at_ms))?;
            self.put(ScheduleKind::Retry, bucket, dispatch_id, at_ms);
            Ok(())
        })
    }

    fn remove_from_retry<'a>(
        &'a self,
        bucket: u32,
        dispatch_id: &'a str,
    ) -> BoxFuture<'a, Result<(), ScheduleError>> {
        Box::pin(async move {
            self.record("remove_from_retry", bucket, dispatch_id, None)?;
            self.take(ScheduleKind::Retry, dispatch_id);
            Ok(())
        })
    }

    fn remove_from_all<'a>(
        &'a self,
        bucket: u32,
        dispatch_id: &'a str,
    ) -> BoxFuture<'a, Result<(), ScheduleError>> {
        Box::pin(async move {
            self.record("remove_from_all", bucket, dispatch_id, None)?;
            self.take(ScheduleKind::Schedule, dispatch_id);
            self.take(ScheduleKind::Retry, dispatch_id);
            Ok(())
        })
    }

    fn get_due(
        &self,
        kind: ScheduleKind,
        bucket: u32,
        now_ms: i64,
        limit: u32,
    ) -> BoxFuture<'_, Result<Vec<String>, ScheduleError>> {
        Box::pin(async move {
            self.record("get_due", bucket, "", Some(now_ms))?;
            let mut ids = self.ids(kind, bucket, Some(now_ms));
            ids.truncate(limit as usize);
            Ok(ids)
        })
    }

    fn count(&self, kind: ScheduleKind, bucket: u32) -> BoxFuture<'_, Result<u64, ScheduleError>> {
        Box::pin(async move {
            self.record("count", bucket, "", None)?;
            Ok(self.ids(kind, bucket, None).len() as u64)
        })
    }

    fn batch_add_to_schedule(
        &self,
        entries: Vec<ScheduleEntry>,
    ) -> BoxFuture<'_, Result<(), ScheduleError>> {
        Box::pin(async move {
            for entry in &entries {
                self.record(
                    "batch_add_to_schedule",
                    entry.bucket,
                    &entry.dispatch_id,
                    Some(entry.at_ms),
                )?;
            }
            for entry in &entries {
                self.put(
                    ScheduleKind::Schedule,
                    entry.bucket,
                    &entry.dispatch_id,
                    entry.at_ms,
                );
            }
            Ok(())
        })
    }

    fn members(
        &self,
        kind: ScheduleKind,
        bucket: u32,
    ) -> BoxFuture<'_, Result<Vec<String>, ScheduleError>> {
        Box::pin(async move {
            self.record("members", bucket, "", None)?;
            Ok(self.ids(kind, bucket, None))
        })
    }

    fn try_lock<'a>(
        &'a self,
        bucket: u32,
        dispatch_id: &'a str,
        ttl_secs: u64,
    ) -> BoxFuture<'a, Result<DispatchLock, ScheduleError>> {
        Box::pin(async move {
            self.record("try_lock", bucket, dispatch_id, Some(ttl_secs as i64))?;
            let key = format!("seq:dispatch:{{{bucket}}}:lock:{dispatch_id}");
            if !self.locks.lock().unwrap().insert(key.clone()) {
                return Err(ScheduleError::LockAcquisition { key });
            }
            let locks = Arc::clone(&self.locks);
            let held = key.clone();
            Ok(DispatchLock::new(
                key,
                Box::new(move || {
                    locks.lock().unwrap().remove(&held);
                }),
            ))
        })
    }
}

// ---------------------------------------------------------------------------
// Fakes of the injectable dependencies
// ---------------------------------------------------------------------------

/// Ids handed out in order; after the list, `fallback` forever.
pub struct FixedIds {
    queue: Mutex<VecDeque<String>>,
    fallback: String,
}

impl FixedIds {
    pub fn new(fallback: &str) -> Arc<FixedIds> {
        FixedIds::sequence(&[], fallback)
    }

    /// Integer ids (decimal strings), then `fallback` forever.
    pub fn ints(ids: &[i64], fallback: i64) -> Arc<FixedIds> {
        Arc::new(FixedIds {
            queue: Mutex::new(ids.iter().map(|n| id(*n)).collect()),
            fallback: id(fallback),
        })
    }

    pub fn sequence(ids: &[&str], fallback: &str) -> Arc<FixedIds> {
        Arc::new(FixedIds {
            queue: Mutex::new(ids.iter().map(|s| (*s).to_owned()).collect()),
            fallback: fallback.to_owned(),
        })
    }
}

impl IdGenerator for FixedIds {
    fn next_id(&self) -> String {
        self.queue
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| self.fallback.clone())
    }
}

/// `getContactInboxes` mock: returns a fixed list, records `(ws, contact)`.
#[derive(Default)]
pub struct FakeContactInboxLookup {
    inboxes: Mutex<Vec<ContactInboxRef>>,
    calls: Mutex<Vec<(String, String)>>,
}

impl FakeContactInboxLookup {
    pub fn new(ids: &[&str]) -> Arc<FakeContactInboxLookup> {
        let fake = FakeContactInboxLookup::default();
        fake.set(ids);
        Arc::new(fake)
    }

    pub fn set(&self, ids: &[&str]) {
        *self.inboxes.lock().unwrap() = ids
            .iter()
            .map(|id| ContactInboxRef {
                id: (*id).to_owned(),
            })
            .collect();
    }

    /// [`FakeContactInboxLookup::set`] with integer ids.
    pub fn set_ints(&self, ids: &[i64]) {
        let ids: Vec<String> = ids.iter().map(|n| id(*n)).collect();
        let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
        self.set(&refs);
    }

    pub fn calls(&self) -> Vec<(String, String)> {
        self.calls.lock().unwrap().clone()
    }
}

impl ContactInboxLookup for FakeContactInboxLookup {
    fn contact_inboxes(
        &self,
        _conn: &Connection,
        workspace_id: &str,
        contact_id: &str,
    ) -> Result<Vec<ContactInboxRef>, SequenceError> {
        self.calls
            .lock()
            .unwrap()
            .push((workspace_id.to_owned(), contact_id.to_owned()));
        Ok(self.inboxes.lock().unwrap().clone())
    }
}

/// `createDispatch` mock: records the params, returns a fixed record (or
/// error). It also records whether it ran inside a transaction.
pub struct RecordingDispatchCreator {
    calls: Mutex<Vec<CreateDispatchParams>>,
    in_tx: Mutex<Vec<bool>>,
    result: Mutex<Result<DispatchRecord, SequenceError>>,
}

impl RecordingDispatchCreator {
    pub fn returning(record: DispatchRecord) -> Arc<RecordingDispatchCreator> {
        Arc::new(RecordingDispatchCreator {
            calls: Mutex::new(Vec::new()),
            in_tx: Mutex::new(Vec::new()),
            result: Mutex::new(Ok(record)),
        })
    }

    pub fn set_result(&self, result: Result<DispatchRecord, SequenceError>) {
        *self.result.lock().unwrap() = result;
    }

    pub fn calls(&self) -> Vec<CreateDispatchParams> {
        self.calls.lock().unwrap().clone()
    }

    pub fn in_tx(&self) -> Vec<bool> {
        self.in_tx.lock().unwrap().clone()
    }
}

impl DispatchCreator for RecordingDispatchCreator {
    fn create_dispatch(
        &self,
        conn: &Connection,
        params: &CreateDispatchParams,
    ) -> Result<DispatchRecord, SequenceError> {
        self.calls.lock().unwrap().push(params.clone());
        self.in_tx.lock().unwrap().push(!conn.is_autocommit());
        self.result.lock().unwrap().clone()
    }
}

/// `calculateNextRunAtFromStep` returns a fixed time;
/// `calculateNextValidSendTime` passes the time through.
pub struct FixedNextRun {
    pub next_run_at_ms: i64,
}

impl NextRunPolicy for FixedNextRun {
    fn next_run_at_from_step(&self, _step: &StepTiming, _base_ms: i64) -> i64 {
        self.next_run_at_ms
    }

    fn next_valid_send_time(&self, at_ms: i64, _window: &SendWindow) -> i64 {
        at_ms
    }
}

/// `insert(...).returning(...)` mock: records each row, returns a fixed
/// result.
pub struct FakeInserter {
    rows: Mutex<Vec<NewDispatchRow>>,
    result: Mutex<Result<Option<DispatchRecord>, SequenceError>>,
}

impl FakeInserter {
    pub fn returning(result: Result<Option<DispatchRecord>, SequenceError>) -> Arc<FakeInserter> {
        Arc::new(FakeInserter {
            rows: Mutex::new(Vec::new()),
            result: Mutex::new(result),
        })
    }

    pub fn rows(&self) -> Vec<NewDispatchRow> {
        self.rows.lock().unwrap().clone()
    }

    pub fn calls(&self) -> usize {
        self.rows.lock().unwrap().len()
    }
}

impl DispatchInserter for FakeInserter {
    fn insert_returning(
        &self,
        _conn: &Connection,
        row: &NewDispatchRow,
    ) -> Result<Option<DispatchRecord>, SequenceError> {
        self.rows.lock().unwrap().push(row.clone());
        self.result.lock().unwrap().clone()
    }
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// A `SequenceCtx` on a migrated in-memory database with every dependency
/// faked: `RecordingSchedule`, `TxProbe`, ids `test-id`,
/// `FakeContactInboxLookup` (`inbox-1`), `RecordingDispatchCreator`
/// (`FAKE_DISPATCH`), `FakeInserter` and the real-shape `FixedNextRun`
/// (`next_run_at_ms` given).
pub struct Harness {
    pub db: Arc<Db>,
    pub clock: Arc<TestClock>,
    pub order: OrderLog,
    pub probe: Arc<TxProbe>,
    pub schedule: Arc<RecordingSchedule>,
    pub lookup: Arc<FakeContactInboxLookup>,
    pub creator: Arc<RecordingDispatchCreator>,
    pub inserter: Arc<FakeInserter>,
    pub ctx: SequenceCtx,
}

pub struct HarnessOptions {
    pub now_ms: i64,
    pub ids: Arc<dyn IdGenerator>,
    pub inboxes: Vec<String>,
    pub dispatch: DispatchRecord,
    pub next_run_at_ms: i64,
}

impl Default for HarnessOptions {
    fn default() -> HarnessOptions {
        HarnessOptions {
            now_ms: 1_700_000_000_000,
            ids: FixedIds::new("test-id"),
            inboxes: vec![id(INBOX_1)],
            dispatch: DispatchRecord {
                id: "dispatch-1".to_owned(),
                bucket: 42,
                run_at_ms: 1_700_000_000_000,
            },
            next_run_at_ms: 1_700_000_000_000,
        }
    }
}

pub async fn harness(options: HarnessOptions) -> Harness {
    let db = crate::support::migrated_db().await;
    let clock = TestClock::new(options.now_ms);
    let order = order_log();
    let probe = TxProbe::new(&order);
    let schedule = RecordingSchedule::new(&order);
    let inbox_ids: Vec<&str> = options.inboxes.iter().map(String::as_str).collect();
    let lookup = FakeContactInboxLookup::new(&inbox_ids);
    let creator = RecordingDispatchCreator::returning(options.dispatch.clone());
    let inserter = FakeInserter::returning(Ok(Some(options.dispatch.clone())));
    let ctx = SequenceCtx {
        db: Arc::clone(&db),
        schedule: schedule.clone(),
        clock: clock.clone(),
        ids: options.ids,
        observer: Some(probe.clone()),
        contact_inboxes: lookup.clone(),
        dispatches: creator.clone(),
        inserter: inserter.clone(),
        next_run: Arc::new(FixedNextRun {
            next_run_at_ms: options.next_run_at_ms,
        }),
    };
    Harness {
        db,
        clock,
        order,
        probe,
        schedule,
        lookup,
        creator,
        inserter,
        ctx,
    }
}

impl Harness {
    pub fn order(&self) -> Vec<String> {
        self.order.lock().unwrap().clone()
    }
}

// ---------------------------------------------------------------------------
// Id mapping (TypeScript string id -> integer id, P2.4 DDL)
// ---------------------------------------------------------------------------
//
// Every P2.4 id is INTEGER; the API returns ids as decimal strings
// (DECISIONS P2.2). Each TypeScript string id of the frozen tests maps to
// one fixed integer below (one TS string = one integer in every file).
// Fake-only tests that never touch the database keep their string ids.

/// Decimal string of an integer id (what the API returns).
pub fn id(n: i64) -> String {
    n.to_string()
}

// Workspaces.
pub const WS_1: i64 = 1; // "ws-1"
pub const WS_2: i64 = 2; // "ws-2"
pub const W1: i64 = 3; // "w1"
pub const W2: i64 = 4; // "w2"
pub const MY_WORKSPACE: i64 = 5; // "my-workspace"
pub const OTHER_WORKSPACE: i64 = 6; // "other-workspace"
// Sequences.
pub const SEQ_1: i64 = 101; // "seq-1"
pub const SEQ_2: i64 = 102; // "seq-2"
pub const SEQ_3: i64 = 103; // "seq-3"
pub const SEQ_ABC: i64 = 104; // "seq-abc"
// Contacts.
pub const CONTACT_1: i64 = 201; // "contact-1"
pub const CONTACT_2: i64 = 202; // "contact-2"
pub const C_1: i64 = 203; // "c-1"
pub const MY_CONTACT: i64 = 204; // "my-contact"
pub const OTHER_CONTACT: i64 = 205; // "other-contact"
// Inboxes ("inbox-*" is also the contact inbox id the mocked
// getContactInboxes returns in the enroll / advance tests).
pub const INBOX_1: i64 = 301; // "inbox-1"
pub const INBOX_2: i64 = 302; // "inbox-2"
pub const INBOX_X: i64 = 303; // "inbox-X"
pub const INBOX_99: i64 = 304; // "inbox-99"
pub const INBOX_OTHER: i64 = 305; // "inbox-other"
// Contact inboxes.
pub const CI_1: i64 = 401; // "ci-1"
pub const CI_9: i64 = 402; // "ci-9"
pub const CI_MINE: i64 = 403; // "ci-mine"
pub const CI_OTHER: i64 = 404; // "ci-other"
/// Parent-only contact inbox of seeded dispatches (no TS id).
pub const CI_PARENT: i64 = 499;
// Steps.
pub const STEP_1: i64 = 501; // "step-1"
pub const STEP_2: i64 = 502; // "step-2"
pub const STEP_XYZ: i64 = 503; // "step-xyz"
pub const STEP_ZERO: i64 = 504; // "step-zero"
pub const STEP_TX: i64 = 505; // "step-tx"
pub const STEP_99: i64 = 506; // "step-99"
pub const S_1: i64 = 511; // "s-1"
pub const S_2: i64 = 512; // "s-2"
// Enrollments (ContactOnSequence).
pub const ENROLLMENT_1: i64 = 601; // "enrollment-1"
pub const ENROLL_1: i64 = 602; // "enroll-1"
pub const ENROLL_2: i64 = 603; // "enroll-2"
pub const ENROLL_42: i64 = 604; // "enroll-42"
pub const E1: i64 = 605; // "e1"
pub const E2: i64 = 606; // "e2"
pub const MY_ENROLLMENT: i64 = 607; // "my-enrollment"
pub const OTHER_ENROLLMENT: i64 = 608; // "other-enrollment"
pub const EXISTING_ENROLLMENT: i64 = 609; // the pre-seeded conflicting enrollment
pub const COS_1: i64 = 611; // "cos-1"
pub const COS_2: i64 = 612; // "cos-2"
pub const COS_3: i64 = 613; // "cos-3"
/// Enrollment of the other-workspace decoys: an enrollment id belongs to one
/// workspace (composite FK), so the decoy needs its own enrollment.
pub const E1_W2: i64 = 614;
pub const ENROLL_1_WS_2: i64 = 615;
pub const MY_ENROLLMENT_OTHER_WS: i64 = 616;
pub const MISSING_ID: i64 = 999; // "missing-id"
// Dispatches.
pub const D_1: i64 = 701; // "d-1"
pub const D_2: i64 = 702; // "d-2"
pub const D1: i64 = 703; // "d1"
pub const D2: i64 = 704; // "d2"
pub const D_TX: i64 = 705; // "d-tx"
pub const D_OTHER_WS: i64 = 706; // "d-other-ws"
pub const D_RUNNING: i64 = 707; // "d-running"
pub const D_MATCH: i64 = 708; // "d-match"
pub const D_OTHER_ENROLLMENT: i64 = 709; // "d-other-enrollment"
pub const DECOY_ENROLLMENT: i64 = 711; // "decoy-enrollment"
pub const DECOY_WORKSPACE: i64 = 712; // "decoy-workspace"
pub const DECOY_STATUS: i64 = 713; // "decoy-status"
/// "d-0".."d-4" = `D_N + n`.
pub const D_N: i64 = 720;
pub const DISPATCH_1: i64 = 801; // "dispatch-1"
pub const DISPATCH_2: i64 = 802; // "dispatch-2"
/// Owner of every seeded workspace (parent only).
pub const OWNER_USER: i64 = 1;

// ---------------------------------------------------------------------------
// Row seeding and reading (P2.4 tables)
// ---------------------------------------------------------------------------

fn to_sql(value: &Value) -> SqlValue {
    match value {
        Value::Null => SqlValue::Null,
        Value::Bool(b) => SqlValue::Integer(i64::from(*b)),
        Value::Number(n) => match n.as_i64() {
            Some(i) => SqlValue::Integer(i),
            None => SqlValue::Real(n.as_f64().unwrap_or_default()),
        },
        Value::String(s) => SqlValue::Text(s.clone()),
        other => SqlValue::Text(other.to_string()),
    }
}

fn from_sql(value: SqlValue) -> Value {
    match value {
        SqlValue::Null => Value::Null,
        SqlValue::Integer(i) => json!(i),
        SqlValue::Real(f) => json!(f),
        SqlValue::Text(s) => json!(s),
        SqlValue::Blob(b) => json!(b),
    }
}

async fn insert_with(db: &Db, verb: &'static str, table: &str, row: Value) {
    let object: Map<String, Value> = row.as_object().cloned().expect("row object");
    let columns: Vec<String> = object.keys().map(|k| format!("\"{k}\"")).collect();
    let marks: Vec<&str> = object.keys().map(|_| "?").collect();
    let sql = format!(
        "{verb} INTO \"{table}\" ({}) VALUES ({})",
        columns.join(", "),
        marks.join(", ")
    );
    let values: Vec<SqlValue> = object.values().map(to_sql).collect();
    db.write(move |tx| {
        tx.execute(&sql, chatbotx_db::rusqlite::params_from_iter(values))?;
        Ok(())
    })
    .await
    .unwrap_or_else(|e| panic!("seed {table}: {e:?}"));
}

/// `INSERT INTO "<table>" (<keys>) VALUES (..)` from a JSON object
/// (booleans as 0/1, `null` as NULL). Panics on a database error.
pub async fn insert_row(db: &Db, table: &str, row: Value) {
    insert_with(db, "INSERT", table, row).await;
}

/// `INSERT OR IGNORE` variant, for parent rows.
async fn ensure_row(db: &Db, table: &str, row: Value) {
    insert_with(db, "INSERT OR IGNORE", table, row).await;
}

/// The row of `table` with `"id" = id` as a JSON object (`None` = no row).
pub async fn read_row(db: &Db, table: &str, id: i64) -> Option<Value> {
    let sql = format!("SELECT * FROM \"{table}\" WHERE \"id\" = ?1");
    db.read(move |conn| {
        let mut stmt = conn.prepare(&sql)?;
        let names: Vec<String> = stmt.column_names().map(|s| s.to_string()).collect();
        let mut rows = stmt.query([id])?;
        match rows.next()? {
            None => Ok(None),
            Some(row) => {
                let mut object = Map::new();
                for (i, name) in names.iter().enumerate() {
                    object.insert(name.clone(), from_sql(row.get::<_, SqlValue>(i)?));
                }
                Ok(Some(Value::Object(object)))
            }
        }
    })
    .await
    .unwrap_or_else(|e| panic!("read {table}: {e:?}"))
}

/// `SELECT count(*) FROM "<table>"`.
pub async fn count_rows(db: &Db, table: &str) -> i64 {
    let sql = format!("SELECT count(*) FROM \"{table}\"");
    db.read(move |conn| Ok(conn.query_row(&sql, [], |r| r.get::<_, i64>(0))?))
        .await
        .unwrap_or_else(|e| panic!("count {table}: {e:?}"))
}

/// One column of a row (`Value::Null` when the row or column is missing).
pub async fn column(db: &Db, table: &str, id: i64, name: &str) -> Value {
    read_row(db, table, id)
        .await
        .and_then(|row| row.get(name).cloned())
        .unwrap_or(Value::Null)
}

// Parent rows (INSERT OR IGNORE). None of them writes a row into a table a
// test counts: they write "User", "Workspace", "Contact", "Inbox",
// "ContactInbox" (parent-only ids), "Sequence", "SequenceStep" (inactive,
// parent-only) and, for dispatch seeds only, "ContactOnSequence".

/// `"User"` `OWNER_USER` and `"Workspace"` `ws` (tenant 1 exists after migrate).
pub async fn ensure_workspace(db: &Db, ws: i64) {
    ensure_row(
        db,
        "User",
        json!({ "id": OWNER_USER, "email": "owner@example.test" }),
    )
    .await;
    ensure_row(
        db,
        "Workspace",
        json!({ "id": ws, "name": format!("workspace-{ws}"), "ownerId": OWNER_USER }),
    )
    .await;
}

/// `"Contact"` `contact` in `ws`.
pub async fn ensure_contact(db: &Db, contact: i64, ws: i64) {
    ensure_workspace(db, ws).await;
    ensure_row(db, "Contact", json!({ "id": contact, "workspaceId": ws })).await;
}

/// `"Inbox"` `inbox` in `ws`.
pub async fn ensure_inbox(db: &Db, inbox: i64, ws: i64) {
    ensure_workspace(db, ws).await;
    ensure_row(
        db,
        "Inbox",
        json!({
            "id": inbox, "workspaceId": ws, "name": format!("inbox-{inbox}"),
            "channel": "messenger", "sourceId": format!("src-inbox-{inbox}"),
        }),
    )
    .await;
}

/// `"ContactInbox"` `ci` of `contact` on `inbox` (both created in `ws`).
pub async fn seed_contact_inbox_row(
    db: &Db,
    ci: i64,
    contact: i64,
    inbox: i64,
    ws: i64,
    ignore: bool,
) {
    ensure_contact(db, contact, ws).await;
    ensure_inbox(db, inbox, ws).await;
    let row = json!({
        "id": ci, "contactId": contact, "originalContactId": contact, "inboxId": inbox,
        "channel": "messenger", "source": "messenger", "sourceId": format!("src-ci-{ci}"),
    });
    if ignore {
        ensure_row(db, "ContactInbox", row).await;
    } else {
        insert_row(db, "ContactInbox", row).await;
    }
}

/// Parent-only `"ContactInbox"` `ci` of `contact` in `ws`, on its own inbox
/// (`ci + 10 000`).
pub async fn ensure_contact_inbox(db: &Db, ci: i64, contact: i64, ws: i64) {
    seed_contact_inbox_row(db, ci, contact, ci + 10_000, ws, true).await;
}

/// `"Sequence"` `seq` in `ws`.
pub async fn ensure_sequence(db: &Db, seq: i64, ws: i64) {
    ensure_workspace(db, ws).await;
    ensure_row(
        db,
        "Sequence",
        json!({ "id": seq, "workspaceId": ws, "name": format!("sequence-{seq}") }),
    )
    .await;
}

/// Parent-only `"SequenceStep"` `step` of `seq` (inactive, order 1000, so
/// no step query of a test picks it).
pub async fn ensure_step(db: &Db, step: i64, seq: i64) {
    ensure_row(
        db,
        "SequenceStep",
        json!({ "id": step, "sequenceId": seq, "order": 1000, "delayDays": 0, "isActive": false }),
    )
    .await;
}

/// Parent-only `"ContactOnSequence"` `enrollment` in `ws` (composite FK
/// parent of dispatches), with its own parent contact (`enrollment + 20 000`)
/// so the (contact, sequence, workspace) key never collides.
pub async fn ensure_enrollment(db: &Db, enrollment: i64, ws: i64, seq: i64) {
    let contact = enrollment + 20_000;
    ensure_contact(db, contact, ws).await;
    ensure_sequence(db, seq, ws).await;
    ensure_row(
        db,
        "ContactOnSequence",
        json!({
            "id": enrollment, "workspaceId": ws, "sequenceId": seq,
            "contactId": contact, "status": "active",
        }),
    )
    .await;
}

/// Every parent of a `"SequenceDispatch"` row.
pub async fn ensure_dispatch_parents(
    db: &Db,
    ws: i64,
    enrollment: i64,
    seq: i64,
    contact: i64,
    step: i64,
    ci: i64,
) {
    ensure_workspace(db, ws).await;
    ensure_sequence(db, seq, ws).await;
    ensure_contact(db, contact, ws).await;
    ensure_step(db, step, seq).await;
    ensure_contact_inbox(db, ci, contact, ws).await;
    ensure_enrollment(db, enrollment, ws, seq).await;
}

/// A `"ContactOnSequence"` row (enrollment) with the defaults of
/// `makeActiveEnrollment`, merged with `overrides`. Creates its workspace,
/// contact and sequence.
pub async fn seed_enrollment(db: &Db, overrides: Value) {
    let mut row = json!({
        "id": ENROLLMENT_1,
        "workspaceId": WS_1,
        "sequenceId": SEQ_1,
        "contactId": CONTACT_1,
        "status": "active",
        "lastStepId": null,
        "currentStep": 0,
    });
    merge(&mut row, overrides);
    let ws = int(&row, "workspaceId");
    ensure_contact(db, int(&row, "contactId"), ws).await;
    ensure_sequence(db, int(&row, "sequenceId"), ws).await;
    insert_row(db, "ContactOnSequence", row).await;
}

/// A `"SequenceStep"` row (active, no delay, anytime) merged with
/// `overrides`. Creates its sequence (in `WS_1`).
pub async fn seed_step(db: &Db, overrides: Value) {
    let mut row = json!({
        "id": STEP_1,
        "sequenceId": SEQ_1,
        "order": 0,
        "isActive": true,
        "delayDays": 0,
        "delayMinutes": 0,
        "delayUnit": null,
        "specificDateTime": null,
        "anytime": true,
        "sendTimeStart": null,
        "sendTimeEnd": null,
        "sendDays": null,
    });
    merge(&mut row, overrides);
    ensure_sequence(db, int(&row, "sequenceId"), WS_1).await;
    insert_row(db, "SequenceStep", row).await;
}

/// A `"SequenceDispatch"` row (pending) merged with `overrides`. Creates
/// every parent row first.
pub async fn seed_dispatch(db: &Db, overrides: Value) {
    let mut row = json!({
        "id": D_1,
        "workspaceId": WS_1,
        "sequenceId": SEQ_1,
        "contactId": C_1,
        "contactInboxId": CI_PARENT,
        "stepId": S_1,
        "enrollmentId": ENROLL_1,
        "bucket": 1,
        "runAtMs": 1_700_000_000_000_i64,
        "status": "pending",
    });
    merge(&mut row, overrides);
    let dispatch_id = int(&row, "id");
    if row.get("idempotencyKey").is_none() {
        row["idempotencyKey"] = json!(format!("key-{dispatch_id}"));
    }
    ensure_dispatch_parents(
        db,
        int(&row, "workspaceId"),
        int(&row, "enrollmentId"),
        int(&row, "sequenceId"),
        int(&row, "contactId"),
        int(&row, "stepId"),
        int(&row, "contactInboxId"),
    )
    .await;
    insert_row(db, "SequenceDispatch", row).await;
}

fn int(row: &Value, key: &str) -> i64 {
    row[key]
        .as_i64()
        .unwrap_or_else(|| panic!("seed column {key} must be an integer id"))
}

fn merge(row: &mut Value, overrides: Value) {
    if let (Some(target), Some(source)) = (row.as_object_mut(), overrides.as_object()) {
        for (k, v) in source {
            target.insert(k.clone(), v.clone());
        }
    }
}

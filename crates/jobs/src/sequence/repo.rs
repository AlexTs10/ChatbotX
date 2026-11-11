//! Shared plumbing of the sequence business functions
//! (`packages/sequence-scheduler`): the error type, the context with its
//! injectable dependencies, the transaction observer (test probe) and the
//! row types. The tables are the P2.4 `SequenceDispatch`,
//! `ContactsOnSequence`, `SequenceStep`, `Inbox` and `ContactInbox`.
//!
//! Database access: every function has a sync `*_in(conn, ..)` core that
//! runs on a caller connection (a `WriteTx` of `Db::write`, the TypeScript
//! "provided client"), and an async wrapper on [`SequenceCtx`] that opens
//! its own write transaction when the TypeScript code used
//! `db.transaction`.

use std::fmt;
use std::sync::Arc;

use chatbotx_db::rusqlite::Connection;
use chatbotx_db::{Db, DbError};

use crate::clock::Clock;
use crate::sequence::dispatch::{CreateDispatchParams, DispatchInserter, DispatchRecord};
use crate::sequence::next_run::StepTiming;
use crate::sequence::schedule::{DispatchSchedule, ScheduleError};
use crate::sequence::send_time::SendWindow;

/// Errors of the sequence business functions. `Display` is the TypeScript
/// `Error.message` (for example `Enrollment missing-id not found`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SequenceError {
    /// A row the caller named does not exist; the text is the full message.
    NotFound(String),
    /// A database error; the text is the database message.
    Db(String),
    /// The dispatch schedule failed.
    Schedule(ScheduleError),
    /// `Failed to remove sequence dispatches from schedule`: the dispatches
    /// whose removal failed, each shown as `<id> bucket=<bucket>`.
    ScheduleRemoval { failed: Vec<(String, u32)> },
    /// Any other error, with its message (`Failed to create dispatch`, ...).
    Other(String),
}

impl fmt::Display for SequenceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        todo!()
    }
}

impl std::error::Error for SequenceError {}

impl From<DbError> for SequenceError {
    fn from(error: DbError) -> SequenceError {
        todo!()
    }
}

impl From<ScheduleError> for SequenceError {
    fn from(error: ScheduleError) -> SequenceError {
        todo!()
    }
}

/// `createId()` (`@chatbotx.io/utils`).
pub trait IdGenerator: Send + Sync {
    fn next_id(&self) -> String;
}

/// The production id generator.
#[derive(Debug, Clone, Copy, Default)]
pub struct CreateId;

impl IdGenerator for CreateId {
    fn next_id(&self) -> String {
        todo!()
    }
}

/// Kind of one SQL statement a business function runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StatementKind {
    Select,
    Insert,
    Update,
    Delete,
}

/// Observer of the transactions and statements of the sequence code (the
/// test `TxProbe`; `None` in production).
///
/// - `on_begin` / `on_commit`: a function opened (and committed) its own
///   multi-statement transaction (the TypeScript `db.transaction(cb)`), or
///   the SQLite schedule ran one write transaction. A single statement on a
///   caller or root client is not reported as a transaction.
/// - `on_statement`: one statement on `table` (Drizzle table name).
pub trait TxObserver: Send + Sync {
    fn on_begin(&self);
    fn on_commit(&self);
    fn on_statement(&self, kind: StatementKind, table: &str);
}

/// Which client an async function uses (the TypeScript `client` option).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ClientMode {
    /// No client passed: the function opens its own transaction where the
    /// TypeScript code calls `db.transaction`.
    #[default]
    Default,
    /// The root `db` passed as the client: no wrapping transaction; every
    /// statement commits on its own.
    Root,
}

/// A contact inbox the dispatches go to (`{ id }`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContactInboxRef {
    pub id: String,
}

/// `getContactInboxes(workspaceId, contactId)` as an injectable dependency.
pub trait ContactInboxLookup: Send + Sync {
    fn contact_inboxes(
        &self,
        conn: &Connection,
        workspace_id: &str,
        contact_id: &str,
    ) -> Result<Vec<ContactInboxRef>, SequenceError>;
}

/// The SQL lookup (`contacts::get_contact_inboxes_in`).
#[derive(Clone, Default)]
pub struct SqlContactInboxLookup {
    pub observer: Option<Arc<dyn TxObserver>>,
}

impl ContactInboxLookup for SqlContactInboxLookup {
    fn contact_inboxes(
        &self,
        conn: &Connection,
        workspace_id: &str,
        contact_id: &str,
    ) -> Result<Vec<ContactInboxRef>, SequenceError> {
        todo!()
    }
}

/// `createDispatch(params)` as an injectable dependency. Runs on the
/// connection of the caller (inside its transaction).
pub trait DispatchCreator: Send + Sync {
    fn create_dispatch(
        &self,
        conn: &Connection,
        params: &CreateDispatchParams,
    ) -> Result<DispatchRecord, SequenceError>;
}

/// The SQL `createDispatch` (`dispatch::create_dispatch_in`).
#[derive(Clone)]
pub struct SqlDispatchCreator {
    pub inserter: Arc<dyn DispatchInserter>,
    pub ids: Arc<dyn IdGenerator>,
}

impl DispatchCreator for SqlDispatchCreator {
    fn create_dispatch(
        &self,
        conn: &Connection,
        params: &CreateDispatchParams,
    ) -> Result<DispatchRecord, SequenceError> {
        todo!()
    }
}

/// `calculateNextRunAtFromStep` + `calculateNextValidSendTime` as an
/// injectable dependency of `advanceEnrollment`.
pub trait NextRunPolicy: Send + Sync {
    fn next_run_at_from_step(&self, step: &StepTiming, base_ms: i64) -> i64;
    fn next_valid_send_time(&self, at_ms: i64, window: &SendWindow) -> i64;
}

/// The real functions (`next_run`, `send_time` in UTC).
#[derive(Debug, Clone, Copy, Default)]
pub struct DefaultNextRunPolicy;

impl NextRunPolicy for DefaultNextRunPolicy {
    fn next_run_at_from_step(&self, step: &StepTiming, base_ms: i64) -> i64 {
        todo!()
    }

    fn next_valid_send_time(&self, at_ms: i64, window: &SendWindow) -> i64 {
        todo!()
    }
}

/// Everything the async sequence functions need.
#[derive(Clone)]
pub struct SequenceCtx {
    pub db: Arc<Db>,
    /// The `SchedulerClient` replacement.
    pub schedule: Arc<dyn DispatchSchedule>,
    pub clock: Arc<dyn Clock>,
    pub ids: Arc<dyn IdGenerator>,
    pub observer: Option<Arc<dyn TxObserver>>,
    pub contact_inboxes: Arc<dyn ContactInboxLookup>,
    pub dispatches: Arc<dyn DispatchCreator>,
    /// Row insert used by `dispatch::create_dispatch`.
    pub inserter: Arc<dyn DispatchInserter>,
    pub next_run: Arc<dyn NextRunPolicy>,
}

impl SequenceCtx {
    /// The production context: SQL dependencies, `CreateId`, no observer.
    pub fn new(
        db: Arc<Db>,
        schedule: Arc<dyn DispatchSchedule>,
        clock: Arc<dyn Clock>,
    ) -> SequenceCtx {
        todo!()
    }
}

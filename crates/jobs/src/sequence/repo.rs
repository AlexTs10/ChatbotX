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
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

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
        match self {
            SequenceError::NotFound(m) | SequenceError::Db(m) | SequenceError::Other(m) => {
                f.write_str(m)
            }
            SequenceError::Schedule(e) => write!(f, "{e}"),
            SequenceError::ScheduleRemoval { failed } => {
                let list: Vec<String> = failed
                    .iter()
                    .map(|(id, bucket)| format!("{id} bucket={bucket}"))
                    .collect();
                write!(
                    f,
                    "Failed to remove sequence dispatches from schedule: {}",
                    list.join(", ")
                )
            }
        }
    }
}

impl std::error::Error for SequenceError {}

impl From<DbError> for SequenceError {
    fn from(error: DbError) -> SequenceError {
        SequenceError::Db(error.to_string())
    }
}

impl From<ScheduleError> for SequenceError {
    fn from(error: ScheduleError) -> SequenceError {
        SequenceError::Schedule(error)
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
    /// A time-ordered integer id (unix ms in the high bits, a counter in the
    /// low 20 bits), unique in this process.
    fn next_id(&self) -> String {
        static LAST: AtomicI64 = AtomicI64::new(0);
        let now = crate::clock::SystemClock.now_ms().max(0) << 20;
        let mut last = LAST.load(Ordering::Relaxed);
        loop {
            let next = now.max(last + 1);
            match LAST.compare_exchange_weak(last, next, Ordering::Relaxed, Ordering::Relaxed) {
                Ok(_) => return next.to_string(),
                Err(current) => last = current,
            }
        }
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
        Ok(crate::sequence::contacts::get_contact_inboxes_in(
            conn,
            self.observer.as_deref(),
            workspace_id,
            contact_id,
        )?
        .into_iter()
        .map(|row| ContactInboxRef { id: row.id })
        .collect())
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
        crate::sequence::dispatch::create_dispatch_in(
            conn,
            self.inserter.as_ref(),
            self.ids.as_ref(),
            params,
        )
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
        crate::sequence::next_run::calculate_next_run_at_from_step(
            step,
            Some(base_ms),
            &crate::clock::SystemClock,
        )
    }

    fn next_valid_send_time(&self, at_ms: i64, window: &SendWindow) -> i64 {
        crate::sequence::send_time::calculate_next_valid_send_time(at_ms, window, chrono_tz::UTC)
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
        let inserter: Arc<dyn DispatchInserter> =
            Arc::new(crate::sequence::dispatch::SqlDispatchInserter);
        let ids: Arc<dyn IdGenerator> = Arc::new(CreateId);
        SequenceCtx {
            db,
            schedule,
            clock,
            ids: Arc::clone(&ids),
            observer: None,
            contact_inboxes: Arc::new(SqlContactInboxLookup::default()),
            dispatches: Arc::new(SqlDispatchCreator {
                inserter: Arc::clone(&inserter),
                ids,
            }),
            inserter,
            next_run: Arc::new(DefaultNextRunPolicy),
        }
    }

    pub(crate) fn note(&self, kind: StatementKind, table: &str) {
        note(self.observer.as_deref(), kind, table);
    }

    pub(crate) fn begin(&self) {
        if let Some(o) = &self.observer {
            o.on_begin();
        }
    }

    pub(crate) fn commit(&self) {
        if let Some(o) = &self.observer {
            o.on_commit();
        }
    }
}

/// Reports one statement to `observer`.
pub(crate) fn note(observer: Option<&dyn TxObserver>, kind: StatementKind, table: &str) {
    if let Some(o) = observer {
        o.on_statement(kind, table);
    }
}

/// An id as the integer the P2.4 tables store (`"601"` → 601).
pub fn parse_id(id: &str) -> Result<i64, SequenceError> {
    id.trim()
        .parse()
        .map_err(|_| SequenceError::Other(format!("invalid id {id:?}")))
}

/// Runs `f` on a reader connection; its `SequenceError` comes back unchanged.
pub(crate) async fn read_tx<R, F>(db: &Db, f: F) -> Result<R, SequenceError>
where
    F: FnOnce(&Connection) -> Result<R, SequenceError> + Send + 'static,
    R: Send + 'static,
{
    db.read(move |conn| Ok(f(conn))).await?
}

/// Runs `f` in one write transaction; an `Err` of `f` rolls back and comes
/// back unchanged.
pub(crate) async fn write_tx<R, F>(db: &Db, f: F) -> Result<R, SequenceError>
where
    F: FnOnce(&Connection) -> Result<R, SequenceError> + Send + 'static,
    R: Send + 'static,
{
    let slot: Arc<Mutex<Option<SequenceError>>> = Arc::new(Mutex::new(None));
    let inner = Arc::clone(&slot);
    let result = db
        .write(move |tx| match f(tx) {
            Ok(value) => Ok(value),
            Err(err) => {
                *inner.lock().unwrap_or_else(PoisonError::into_inner) = Some(err);
                Err(DbError::other("rolled back"))
            }
        })
        .await;
    match result {
        Ok(value) => Ok(value),
        Err(db_err) => Err(slot
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
            .unwrap_or_else(|| SequenceError::from(db_err))),
    }
}

/// `rusqlite` errors of a statement, mapped like the writer maps them.
impl From<chatbotx_db::rusqlite::Error> for SequenceError {
    fn from(error: chatbotx_db::rusqlite::Error) -> SequenceError {
        SequenceError::from(DbError::from(error))
    }
}

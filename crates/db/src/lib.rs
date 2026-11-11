//! Database layer (P2.3, `conversion/specs/P2.3.md`).
//!
//! One SQLite file (rusqlite, bundled SQLite) with one writer thread and a
//! small pool of read-only connections, the embedded migration runner, the
//! app SQL functions (`lower_u`, `ilike`, `regexp`, `regexp_replace`,
//! `tz_*`, `vector_distance_cos`) and the error mapping to Postgres SQLSTATE
//! codes.
//!
//! The API below is DECLARED ONLY (the P2.3 test porter wrote it from the
//! spec so that the ported tests in `crates/db/tests/` compile). Every body is
//! `todo!()`. The P2.3 implementer fills it in and may change private parts;
//! the public names and signatures are what the tests use.

use std::fmt;
use std::marker::PhantomData;
use std::ops::Deref;
use std::path::Path;
use std::time::{Duration, Instant};

pub use rusqlite;

// ---------------------------------------------------------------------------
// Db: connection model (spec section 2)
// ---------------------------------------------------------------------------

/// Handle to the database: one writer thread + `DB_READERS` (default 3)
/// read-only connections, all set up as in spec section 2.2 / 2.3.
pub struct Db {
    _private: (),
}

impl Db {
    /// Opens (creates if needed) the SQLite file at `path`. Runs the
    /// per-connection setup of spec 2.2 on every connection. Does not migrate.
    pub fn open(_path: impl AsRef<Path>) -> Result<Db, DbError> {
        todo!("P2.3: open writer thread + reader pool (spec 2.1-2.3)")
    }

    /// Opens a private in-memory database (shared between the writer and
    /// the readers of this `Db` only). Does not migrate.
    pub fn open_in_memory() -> Result<Db, DbError> {
        todo!("P2.3: in-memory database for tests")
    }

    /// Runs [`migrate`] on the writer connection (spec 4.3).
    pub async fn migrate(&self) -> Result<MigrateReport, MigrationError> {
        todo!("P2.3: migration runner on the writer")
    }

    /// Runs `f` on the writer thread inside `BEGIN IMMEDIATE`.
    /// `Ok` commits, `Err` rolls back. A statement timeout set with
    /// [`WriteTx::set_statement_timeout`] ends with the job.
    /// Errors are mapped as in spec 2.4 (SQLSTATE, constraint name, table).
    pub async fn write<F, R>(&self, _f: F) -> Result<R, DbError>
    where
        F: FnOnce(&mut WriteTx<'_>) -> Result<R, DbError> + Send + 'static,
        R: Send + 'static,
    {
        todo!("P2.3: writer job")
    }

    /// Runs `f` on a read-only connection from the pool (`query_only = ON`).
    pub async fn read<F, R>(&self, _f: F) -> Result<R, DbError>
    where
        F: FnOnce(&rusqlite::Connection) -> Result<R, DbError> + Send + 'static,
        R: Send + 'static,
    {
        todo!("P2.3: reader job")
    }
}

/// The open write transaction of one [`Db::write`] job.
/// Derefs to the connection, so every `rusqlite::Connection` method is
/// available (`execute`, `query_row`, `prepare`, ...).
pub struct WriteTx<'conn> {
    _conn: PhantomData<&'conn mut rusqlite::Connection>,
}

impl WriteTx<'_> {
    /// Port of `setLocalStatementTimeout`: installs a progress handler that
    /// interrupts any statement after the deadline, for the rest of this
    /// transaction only (the scope of `SET LOCAL`). An interrupted statement
    /// fails with SQLSTATE `57014`.
    pub fn set_statement_timeout(&mut self, _timeout: StatementTimeout) -> Result<(), DbError> {
        todo!("P2.3: statement timeout (spec 2.3)")
    }

    /// The statement timeout active in this transaction (`show statement_timeout`).
    pub fn statement_timeout(&self) -> Option<Duration> {
        todo!("P2.3: statement timeout (spec 2.3)")
    }

    /// Test hook: the deadline of the installed progress handler, if any.
    pub fn statement_deadline(&self) -> Option<Instant> {
        todo!("P2.3: statement timeout test hook")
    }

    /// Test hook: how many times a progress handler was installed in this
    /// transaction.
    pub fn progress_handler_installs(&self) -> usize {
        todo!("P2.3: statement timeout test hook")
    }
}

impl Deref for WriteTx<'_> {
    type Target = rusqlite::Connection;

    fn deref(&self) -> &rusqlite::Connection {
        todo!("P2.3: WriteTx derefs to the writer connection")
    }
}

// ---------------------------------------------------------------------------
// Statement timeout (replaces packages/database/src/statement-timeout.ts)
// ---------------------------------------------------------------------------

/// A validated statement timeout literal: `^\d+(ms|s)$` (same rule as
/// `STATEMENT_TIMEOUT_LITERAL`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatementTimeout {
    _duration: Duration,
}

impl StatementTimeout {
    /// Parses `"10s"`, `"250ms"`, ... Every other text is an error
    /// (the `RangeError` of the TypeScript guard).
    pub fn parse(_literal: &str) -> Result<StatementTimeout, InvalidStatementTimeout> {
        todo!("P2.3: StatementTimeout::parse")
    }

    pub fn duration(&self) -> Duration {
        todo!("P2.3: StatementTimeout::duration")
    }
}

/// The literal is not `^\d+(ms|s)$`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidStatementTimeout {
    _literal: String,
}

impl fmt::Display for InvalidStatementTimeout {
    fn fmt(&self, _f: &mut fmt::Formatter<'_>) -> fmt::Result {
        todo!("P2.3: InvalidStatementTimeout Display")
    }
}

impl std::error::Error for InvalidStatementTimeout {}

// ---------------------------------------------------------------------------
// Errors (spec 2.4)
// ---------------------------------------------------------------------------

/// A database error with the Postgres-like fields the business code checks.
#[derive(Debug)]
pub struct DbError {
    _private: (),
}

impl DbError {
    /// The SQLSTATE-like code of spec 2.4 (`23505`, `23503`, `23502`,
    /// `23514`, `57014`, `55P03`, `54000`), or `None`.
    pub fn sqlstate(&self) -> Option<&str> {
        todo!("P2.3: DbError::sqlstate")
    }

    /// For `23505`: the unique index / UNIQUE constraint name
    /// (`<table>_pkey` for a primary key). `None` for every other error.
    pub fn constraint(&self) -> Option<&str> {
        todo!("P2.3: DbError::constraint")
    }

    /// The table named in the SQLite message, if any.
    pub fn table(&self) -> Option<&str> {
        todo!("P2.3: DbError::table")
    }

    /// The SQLite message (the Postgres `detail` field).
    pub fn detail(&self) -> Option<&str> {
        todo!("P2.3: DbError::detail")
    }

    /// The error message.
    pub fn message(&self) -> &str {
        todo!("P2.3: DbError::message")
    }
}

impl fmt::Display for DbError {
    fn fmt(&self, _f: &mut fmt::Formatter<'_>) -> fmt::Result {
        todo!("P2.3: DbError Display")
    }
}

impl std::error::Error for DbError {}

impl From<rusqlite::Error> for DbError {
    fn from(_error: rusqlite::Error) -> DbError {
        todo!("P2.3: map rusqlite::Error to DbError (spec 2.4)")
    }
}

/// Port of the return value of `describeDatabaseError` (`src/client.ts`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DatabaseErrorDescription {
    pub code: Option<String>,
    pub constraint: Option<String>,
    pub detail: Option<String>,
    pub message: String,
    pub table: Option<String>,
}

/// Port of `describeDatabaseError`: walks the `source()` chain to the first
/// [`DbError`] and reports its fields; with no `DbError` in the chain, only
/// `message` (the top-level `Display`) is set.
pub fn describe_database_error(_error: &(dyn std::error::Error + 'static)) -> DatabaseErrorDescription {
    todo!("P2.3: describe_database_error")
}

// ---------------------------------------------------------------------------
// Migration runner (spec 4)
// ---------------------------------------------------------------------------

/// What one [`migrate`] run applied.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MigrateReport {
    /// Versions applied by this run, in order. Empty on a second run.
    pub applied: Vec<u32>,
}

/// Why [`migrate`] refused to start or failed (spec 4.3).
#[derive(Debug)]
pub enum MigrationError {
    /// The database has a version this binary does not know.
    UnknownVersion { version: u32 },
    /// An applied version has a different name or checksum.
    ChecksumMismatch { version: u32, expected: String, found: String },
    /// Applied versions have a gap.
    Gap { missing: u32 },
    /// A migration failed and was rolled back.
    Failed { version: u32, name: String, source: DbError },
    /// The runner could not read or write `schema_migrations`.
    Db(DbError),
}

impl fmt::Display for MigrationError {
    fn fmt(&self, _f: &mut fmt::Formatter<'_>) -> fmt::Result {
        todo!("P2.3: MigrationError Display")
    }
}

impl std::error::Error for MigrationError {}

/// Applies the embedded migrations (`crates/db/migrations/NNNN_*.sql`) on
/// `conn` (spec 4.3).
pub fn migrate(_conn: &mut rusqlite::Connection) -> Result<MigrateReport, MigrationError> {
    todo!("P2.3: migration runner (spec 4.3)")
}

// ---------------------------------------------------------------------------
// Schema constants
// ---------------------------------------------------------------------------

pub mod schema {
    /// Name of the partial unique index on `ContactInbox ("inboxId",
    /// "sourceParentUserId") WHERE "sourceParentUserId" IS NOT NULL`.
    pub const CONTACT_INBOX_SOURCE_PARENT_USER_ID_KEY: &str = "ContactInbox_inboxId_sourceParentUserId_key";

    /// `Table.column` of the NOT NULL columns that have no database default
    /// but that the Rust insert code always fills (the 16 Drizzle
    /// `.default(sql`[]`)` columns, sorted). P2.3 implementer: fill this list.
    pub const APP_SIDE_DEFAULTS: &[&str] = &[];
}

// ---------------------------------------------------------------------------
// SQL function helpers (spec 3)
// ---------------------------------------------------------------------------

pub mod functions {
    /// Port of `timezoneCandidates` (`packages/utils/src/timezone.ts`): the
    /// caller's spelling first, then its legacy/canonical alias(es). This is
    /// the candidate list that the SQL function `tz_resolve` uses.
    pub fn timezone_candidates(_name: &str) -> Vec<String> {
        todo!("P2.3: timezone_candidates (spec 3.5)")
    }
}

// ---------------------------------------------------------------------------
// Query builders that P2.3 needs for its frozen tests
// ---------------------------------------------------------------------------

pub mod queries {
    /// A SQL text with its positional (`?`) bind values, in order.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct SqlFragment {
        pub sql: String,
        pub params: Vec<String>,
    }

    /// Port of `zonedDateKey(column, timezone)` (`queries/date-bucket.ts`):
    /// the `YYYY-MM-DD` day key of the unix-ms `column_sql` in `timezone`.
    /// The zone name is only a bind value (`tz_resolve(?)`), never SQL text.
    pub fn zoned_date_key(_column_sql: &str, _timezone: &str) -> SqlFragment {
        todo!("P2.3: zoned_date_key (owner of the full builder: queries unit)")
    }
}

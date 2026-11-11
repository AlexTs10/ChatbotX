//! Database layer (P2.3, `conversion/specs/P2.3.md`).
//!
//! One SQLite file (rusqlite, bundled SQLite) with one writer thread and a
//! small pool of read-only connections, the embedded migration runner, the
//! app SQL functions (`lower_u`, `ilike`, `regexp`, `regexp_replace`,
//! `tz_*`, `vector_distance_cos`) and the error mapping to Postgres SQLSTATE
//! codes.

use std::any::Any;
use std::fmt;
use std::ops::Deref;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use rusqlite::config::DbConfig;
use rusqlite::{Connection, TransactionBehavior};
use tokio::sync::{Semaphore, mpsc, oneshot};

pub use rusqlite;

mod error;
pub mod functions;
mod migrate;
#[cfg(test)]
mod query_plan_tests;
#[cfg(test)]
mod schema_part3_tests;

pub use error::{DatabaseErrorDescription, DbError, describe_database_error};
pub use migrate::{MIGRATIONS, MigrateReport, Migration, MigrationError, migrate};

// ---------------------------------------------------------------------------
// Connection setup (spec 2.2 / 2.3)
// ---------------------------------------------------------------------------

/// Default number of read-only connections (`DB_READERS`).
pub const DEFAULT_READERS: usize = 3;

/// Capacity of the writer job queue.
const WRITER_QUEUE: usize = 256;

/// Progress handler period (SQLite VM instructions between deadline checks).
const PROGRESS_OPS: i32 = 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Role {
    Writer,
    Reader,
}

const WRITER_PRAGMAS: &str = "PRAGMA synchronous = NORMAL;
PRAGMA foreign_keys = ON;
PRAGMA cache_size = -8192;
PRAGMA mmap_size = 0;
PRAGMA temp_store = FILE;
PRAGMA cache_spill = ON;
PRAGMA journal_size_limit = 33554432;
PRAGMA wal_autocheckpoint = 1000;
PRAGMA trusted_schema = OFF;
PRAGMA query_only = OFF;
PRAGMA recursive_triggers = OFF;
PRAGMA soft_heap_limit = 33554432;";

const READER_PRAGMAS: &str = "PRAGMA synchronous = NORMAL;
PRAGMA foreign_keys = ON;
PRAGMA cache_size = -4096;
PRAGMA mmap_size = 0;
PRAGMA temp_store = FILE;
PRAGMA trusted_schema = OFF;
PRAGMA query_only = ON;";

/// Per-connection setup, in the order of spec 2.2 (steps 2-6).
pub(crate) fn setup_connection(conn: &Connection, role: Role) -> rusqlite::Result<()> {
    conn.set_db_config(DbConfig::SQLITE_DBCONFIG_DQS_DDL, false)?;
    conn.set_db_config(DbConfig::SQLITE_DBCONFIG_DQS_DML, false)?;
    conn.set_db_config(DbConfig::SQLITE_DBCONFIG_DEFENSIVE, true)?;
    conn.set_db_config(DbConfig::SQLITE_DBCONFIG_TRUSTED_SCHEMA, false)?;
    functions::register(conn)?;
    conn.busy_timeout(Duration::from_millis(5000))?;
    conn.execute_batch(match role {
        Role::Writer => WRITER_PRAGMAS,
        Role::Reader => READER_PRAGMAS,
    })?;
    conn.set_prepared_statement_cache_capacity(64);
    Ok(())
}

// ---------------------------------------------------------------------------
// Db: connection model (spec section 2)
// ---------------------------------------------------------------------------

type Job = Box<dyn FnOnce(&mut Connection) + Send>;
type JobResult<T> = Result<T, Box<dyn Any + Send>>;

struct ReaderPool {
    idle: Mutex<Vec<Connection>>,
    permits: Arc<Semaphore>,
}

/// Handle to the database: one writer thread + `DB_READERS` (default 3)
/// read-only connections, all set up as in spec section 2.2 / 2.3.
pub struct Db {
    writer: Option<mpsc::Sender<Job>>,
    writer_thread: Option<JoinHandle<()>>,
    readers: Arc<ReaderPool>,
}

impl fmt::Debug for Db {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Db").finish_non_exhaustive()
    }
}

impl Db {
    /// Opens (creates if needed) the SQLite file at `path`. Runs the
    /// per-connection setup of spec 2.2 on every connection. Does not migrate.
    /// The reader count comes from `DB_READERS` (default 3).
    pub fn open(path: impl AsRef<Path>) -> Result<Db, DbError> {
        Db::open_with_readers(path, readers_from_env())
    }

    /// [`Db::open`] with an explicit reader count (at least 1).
    pub fn open_with_readers(path: impl AsRef<Path>, readers: usize) -> Result<Db, DbError> {
        let path = path.as_ref();
        Db::open_target(|| Connection::open(path), readers)
    }

    /// Opens a private in-memory database (shared between the writer and
    /// the readers of this `Db` only). Does not migrate.
    pub fn open_in_memory() -> Result<Db, DbError> {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let uri = format!(
            "file:chatbotx-db-mem-{}-{n}?mode=memory&cache=shared",
            std::process::id()
        );
        Db::open_target(|| Connection::open(&uri), DEFAULT_READERS)
    }

    fn open_target(
        open: impl Fn() -> rusqlite::Result<Connection>,
        readers: usize,
    ) -> Result<Db, DbError> {
        let mut writer = open()?;
        setup_connection(&writer, Role::Writer)?;
        let objects: i64 =
            writer.query_row("SELECT count(*) FROM sqlite_schema", [], |r| r.get(0))?;
        if objects == 0 {
            writer.execute_batch("PRAGMA auto_vacuum = INCREMENTAL")?;
        }
        let _mode: String = writer.query_row("PRAGMA journal_mode = WAL", [], |r| r.get(0))?;
        writer.execute_batch("PRAGMA optimize=0x10002")?;
        writer.set_transaction_behavior(TransactionBehavior::Immediate);

        let count = readers.max(1);
        let mut idle = Vec::with_capacity(count);
        for _ in 0..count {
            let reader = open()?;
            setup_connection(&reader, Role::Reader)?;
            idle.push(reader);
        }

        let (sender, mut receiver) = mpsc::channel::<Job>(WRITER_QUEUE);
        let writer_thread = std::thread::Builder::new()
            .name("db-writer".to_owned())
            .spawn(move || {
                while let Some(job) = receiver.blocking_recv() {
                    job(&mut writer);
                }
                let _ = writer.execute_batch("PRAGMA optimize");
            })
            .map_err(|e| DbError::other(format!("cannot start the db-writer thread: {e}")))?;

        Ok(Db {
            writer: Some(sender),
            writer_thread: Some(writer_thread),
            readers: Arc::new(ReaderPool {
                idle: Mutex::new(idle),
                permits: Arc::new(Semaphore::new(count)),
            }),
        })
    }

    /// Sends `job` to the writer thread and waits for its result.
    async fn run_on_writer<T: Send + 'static>(
        &self,
        job: impl FnOnce(&mut Connection) -> T + Send + 'static,
    ) -> Option<T> {
        let (reply, result) = oneshot::channel::<JobResult<T>>();
        let job: Job = Box::new(move |conn| {
            let _ = reply.send(catch_unwind(AssertUnwindSafe(|| job(conn))));
        });
        self.writer.as_ref()?.send(job).await.ok()?;
        match result.await.ok()? {
            Ok(value) => Some(value),
            Err(panic) => resume_unwind(panic),
        }
    }

    /// Runs [`migrate`] on the writer connection (spec 4.3).
    pub async fn migrate(&self) -> Result<MigrateReport, MigrationError> {
        self.run_on_writer(migrate::migrate)
            .await
            .unwrap_or_else(|| Err(MigrationError::Db(DbError::closed())))
    }

    /// Runs `f` on the writer thread inside `BEGIN IMMEDIATE`.
    /// `Ok` commits, `Err` rolls back. A statement timeout set with
    /// [`WriteTx::set_statement_timeout`] ends with the job.
    /// Errors are mapped as in spec 2.4 (SQLSTATE, constraint name, table).
    pub async fn write<F, R>(&self, f: F) -> Result<R, DbError>
    where
        F: FnOnce(&mut WriteTx<'_>) -> Result<R, DbError> + Send + 'static,
        R: Send + 'static,
    {
        self.run_on_writer(move |conn| run_write(conn, f))
            .await
            .unwrap_or_else(|| Err(DbError::closed()))
    }

    /// Runs `f` on a read-only connection from the pool (`query_only = ON`),
    /// inside one deferred transaction (one WAL snapshot).
    pub async fn read<F, R>(&self, f: F) -> Result<R, DbError>
    where
        F: FnOnce(&rusqlite::Connection) -> Result<R, DbError> + Send + 'static,
        R: Send + 'static,
    {
        let permit = Arc::clone(&self.readers.permits)
            .acquire_owned()
            .await
            .map_err(|_| DbError::closed())?;
        let pool = Arc::clone(&self.readers);
        let joined = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let conn = pool
                .idle
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .pop();
            let Some(conn) = conn else {
                return Ok(Err(DbError::other("no idle reader connection")));
            };
            let outcome = catch_unwind(AssertUnwindSafe(|| run_read(&conn, f)));
            if !conn.is_autocommit() {
                let _ = conn.execute_batch("ROLLBACK");
            }
            pool.idle
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(conn);
            outcome
        })
        .await;
        match joined {
            Ok(Ok(result)) => result,
            Ok(Err(panic)) => resume_unwind(panic),
            Err(e) if e.is_panic() => resume_unwind(e.into_panic()),
            Err(_) => Err(DbError::closed()),
        }
    }
}

impl Drop for Db {
    fn drop(&mut self) {
        // Closing the channel ends the writer loop; the thread closes its connection.
        self.writer.take();
        if let Some(thread) = self.writer_thread.take() {
            let _ = thread.join();
        }
    }
}

fn readers_from_env() -> usize {
    std::env::var("DB_READERS")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(DEFAULT_READERS)
}

fn rollback(conn: &Connection) {
    if !conn.is_autocommit() {
        let _ = conn.execute_batch("ROLLBACK");
    }
}

/// One writer job: `BEGIN IMMEDIATE`, `f`, `COMMIT` on `Ok` / `ROLLBACK` on `Err` or panic.
/// The statement timeout handler is removed before the transaction ends.
fn run_write<F, R>(conn: &mut Connection, f: F) -> Result<R, DbError>
where
    F: FnOnce(&mut WriteTx<'_>) -> Result<R, DbError>,
{
    conn.execute_batch("BEGIN IMMEDIATE")?;
    let mut tx = WriteTx {
        conn,
        timeout: None,
        deadline: None,
        installs: 0,
    };
    let outcome = catch_unwind(AssertUnwindSafe(|| f(&mut tx)));
    let had_handler = tx.installs > 0;
    let conn = tx.conn;
    if had_handler {
        let _ = conn.progress_handler(0, None::<fn() -> bool>);
    }
    match outcome {
        Ok(Ok(value)) => match conn.execute_batch("COMMIT") {
            Ok(()) => Ok(value),
            Err(e) => {
                rollback(conn);
                Err(e.into())
            }
        },
        Ok(Err(mut e)) => {
            rollback(conn);
            e.resolve_constraint(conn);
            Err(e)
        }
        Err(panic) => {
            rollback(conn);
            resume_unwind(panic)
        }
    }
}

fn run_read<F, R>(conn: &Connection, f: F) -> Result<R, DbError>
where
    F: FnOnce(&Connection) -> Result<R, DbError>,
{
    conn.execute_batch("BEGIN DEFERRED")?;
    let result = f(conn);
    if !conn.is_autocommit() {
        conn.execute_batch(if result.is_ok() { "COMMIT" } else { "ROLLBACK" })?;
    }
    result
}

/// The open write transaction of one [`Db::write`] job.
/// Derefs to the connection, so every `rusqlite::Connection` method is
/// available (`execute`, `query_row`, `prepare`, ...).
pub struct WriteTx<'conn> {
    conn: &'conn mut Connection,
    timeout: Option<Duration>,
    deadline: Option<Instant>,
    installs: usize,
}

impl WriteTx<'_> {
    /// Port of `setLocalStatementTimeout`: installs a progress handler that
    /// interrupts any statement after the deadline, for the rest of this
    /// transaction only (the scope of `SET LOCAL`). An interrupted statement
    /// fails with SQLSTATE `57014`.
    pub fn set_statement_timeout(&mut self, timeout: StatementTimeout) -> Result<(), DbError> {
        let duration = timeout.duration();
        let deadline = Instant::now()
            .checked_add(duration)
            .ok_or_else(|| DbError::other("statement timeout is too large"))?;
        self.conn
            .progress_handler(PROGRESS_OPS, Some(move || Instant::now() >= deadline))?;
        self.timeout = Some(duration);
        self.deadline = Some(deadline);
        self.installs += 1;
        Ok(())
    }

    /// The statement timeout active in this transaction (`show statement_timeout`).
    pub fn statement_timeout(&self) -> Option<Duration> {
        self.timeout
    }

    /// Test hook: the deadline of the installed progress handler, if any.
    pub fn statement_deadline(&self) -> Option<Instant> {
        self.deadline
    }

    /// Test hook: how many times a progress handler was installed in this
    /// transaction.
    pub fn progress_handler_installs(&self) -> usize {
        self.installs
    }
}

impl Deref for WriteTx<'_> {
    type Target = rusqlite::Connection;

    fn deref(&self) -> &rusqlite::Connection {
        self.conn
    }
}

// ---------------------------------------------------------------------------
// Statement timeout (replaces packages/database/src/statement-timeout.ts)
// ---------------------------------------------------------------------------

/// A validated statement timeout literal: `^\d+(ms|s)$` (same rule as
/// `STATEMENT_TIMEOUT_LITERAL`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatementTimeout {
    duration: Duration,
}

impl StatementTimeout {
    /// Parses `"10s"`, `"250ms"`, ... Every other text is an error
    /// (the `RangeError` of the TypeScript guard).
    pub fn parse(literal: &str) -> Result<StatementTimeout, InvalidStatementTimeout> {
        let invalid = || InvalidStatementTimeout {
            literal: literal.to_owned(),
        };
        let (digits, millis) = if let Some(d) = literal.strip_suffix("ms") {
            (d, true)
        } else if let Some(d) = literal.strip_suffix('s') {
            (d, false)
        } else {
            return Err(invalid());
        };
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return Err(invalid());
        }
        let value: u64 = digits.parse().map_err(|_| invalid())?;
        let duration = if millis {
            Duration::from_millis(value)
        } else {
            Duration::from_secs(value)
        };
        Ok(StatementTimeout { duration })
    }

    pub fn duration(&self) -> Duration {
        self.duration
    }
}

/// The literal is not `^\d+(ms|s)$`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidStatementTimeout {
    literal: String,
}

impl fmt::Display for InvalidStatementTimeout {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "statement_timeout must be a whole number of ms or s, got {:?}",
            self.literal
        )
    }
}

impl std::error::Error for InvalidStatementTimeout {}

// ---------------------------------------------------------------------------
// Schema constants
// ---------------------------------------------------------------------------

pub mod schema {
    /// Name of the partial unique index on `ContactInbox ("inboxId",
    /// "sourceParentUserId") WHERE "sourceParentUserId" IS NOT NULL`.
    pub const CONTACT_INBOX_SOURCE_PARENT_USER_ID_KEY: &str =
        "ContactInbox_inboxId_sourceParentUserId_key";

    /// `Table.column` of the NOT NULL columns that have no database default
    /// but that the Rust insert code always fills (the 16 Drizzle
    /// `.default(sql`[]`)` columns, sorted). Frozen list:
    /// `schema-default-parity.test.ts` `DEFAULTS_MISSING_FROM_DATABASE`.
    pub const APP_SIDE_DEFAULTS: &[&str] = &[
        "AIAgent.messages",
        "AIAgent.models",
        "AIAgent.tools",
        "AIMCPServer.selectedTools",
        "AutomatedResponse.keywords",
        "Folder.paths",
        "IntegrationInstagram.conversationStarters",
        "IntegrationInstagram.persistentMenus",
        "IntegrationMessenger.conversationStarters",
        "IntegrationMessenger.persistentMenus",
        "IntegrationMessenger.personas",
        "IntegrationWebchat.authorizedDomains",
        "IntegrationWebchat.conversationStarters",
        "IntegrationWebchat.persistentMenus",
        "Trigger.actions",
        "UserPersistentMenu.menus",
    ];

    /// The 4 rollup tables of `0004_rest.sql` (they replace the TimescaleDB continuous
    /// aggregates, same names) with their event table, as `(rollup, events)`. Triggers on
    /// the event table keep each rollup exact (P2.5 spec 1.5). Purge and retention code
    /// deletes the rollup rows of a workspace before its events (spec 1.5.2, doubt 12).
    pub const ROLLUP_TABLES: &[(&str, &str)] = &[
        ("analytics_contact_events_hourly", "AnalyticsContactEvent"),
        (
            "analytics_bot_message_events_hourly",
            "AnalyticsBotMessageEvent",
        ),
        (
            "analytics_conversation_events_hourly",
            "AnalyticsConversationEvent",
        ),
        ("analytics_message_events_hourly", "AnalyticsMessageEvent"),
    ];
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
    /// `/ 1000.0` keeps the day right for times before 1970 (integer division
    /// rounds toward zero).
    pub fn zoned_date_key(column_sql: &str, timezone: &str) -> SqlFragment {
        SqlFragment {
            sql: format!(
                "strftime('%Y-%m-%d', tz_wall_ms({column_sql}, tz_resolve(?)) / 1000.0, 'unixepoch')"
            ),
            params: vec![timezone.to_owned()],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDir(std::path::PathBuf);

    impl TempDir {
        fn new() -> TempDir {
            static N: AtomicU64 = AtomicU64::new(0);
            let dir = std::env::temp_dir().join(format!(
                "chatbotx-db-unit-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&dir).expect("temp dir");
            TempDir(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime")
    }

    #[test]
    fn statement_timeout_literals() {
        assert_eq!(
            StatementTimeout::parse("10s").map(|t| t.duration()),
            Ok(Duration::from_secs(10))
        );
        assert_eq!(
            StatementTimeout::parse("0ms").map(|t| t.duration()),
            Ok(Duration::ZERO)
        );
        for bad in [
            "s",
            "ms",
            "1m",
            "1 ms",
            "+1s",
            "1sms",
            "١s",
            "99999999999999999999s",
        ] {
            assert!(StatementTimeout::parse(bad).is_err(), "{bad}");
        }
        let err = StatementTimeout::parse("10").expect_err("invalid");
        assert_eq!(
            err.to_string(),
            "statement_timeout must be a whole number of ms or s, got \"10\""
        );
    }

    #[test]
    fn zoned_date_key_binds_the_zone() {
        let q = queries::zoned_date_key(r#""T"."at""#, "Asia/Saigon");
        assert_eq!(
            q.sql,
            r#"strftime('%Y-%m-%d', tz_wall_ms("T"."at", tz_resolve(?)) / 1000.0, 'unixepoch')"#
        );
        assert_eq!(q.params, ["Asia/Saigon"]);
    }

    #[test]
    fn connections_have_the_spec_settings() {
        rt().block_on(async {
            let dir = TempDir::new();
            let db = Db::open(dir.0.join("a.db")).expect("open");
            db.migrate().await.expect("migrate");
            let (mode, auto_vacuum, fks, query_only, sync) = db
                .write(|tx| {
                    let s = |sql: &str| {
                        tx.query_row(sql, [], |r| r.get::<_, rusqlite::types::Value>(0))
                    };
                    Ok((
                        s("PRAGMA journal_mode")?,
                        s("PRAGMA auto_vacuum")?,
                        s("PRAGMA foreign_keys")?,
                        s("PRAGMA query_only")?,
                        s("PRAGMA synchronous")?,
                    ))
                })
                .await
                .expect("writer pragmas");
            use rusqlite::types::Value::{Integer, Text};
            assert_eq!(mode, Text("wal".to_owned()));
            assert_eq!(auto_vacuum, Integer(2));
            assert_eq!(
                (fks, query_only, sync),
                (Integer(1), Integer(0), Integer(1))
            );

            let (query_only, cache, fks) = db
                .read(|conn| {
                    let i = |sql: &str| conn.query_row(sql, [], |r| r.get::<_, i64>(0));
                    Ok((
                        i("PRAGMA query_only")?,
                        i("PRAGMA cache_size")?,
                        i("PRAGMA foreign_keys")?,
                    ))
                })
                .await
                .expect("reader pragmas");
            assert_eq!((query_only, cache, fks), (1, -4096, 1));

            // A reader cannot write.
            let err = db
                .read(|conn| Ok(conn.execute(r#"DELETE FROM "Tenant""#, [])?))
                .await
                .expect_err("query_only");
            assert!(err.message().contains("readonly"), "{err}");

            // Double-quoted string literals are off on readers and the writer.
            assert!(
                db.read(|conn| Ok(
                    conn.query_row(r#"SELECT "no_such_column""#, [], |r| r.get::<_, String>(0))?
                ))
                .await
                .is_err()
            );
            assert!(
                db.write(|tx| Ok(
                    tx.query_row(r#"SELECT "no_such_column""#, [], |r| r.get::<_, String>(0))?
                ))
                .await
                .is_err()
            );
            // App functions are registered on readers.
            let lowered = db
                .read(|conn| {
                    Ok(conn.query_row("SELECT lower_u('ĐẶNG')", [], |r| r.get::<_, String>(0))?)
                })
                .await
                .expect("lower_u");
            assert_eq!(lowered, "đặng");
        });
    }

    #[test]
    fn write_commits_on_ok_and_rolls_back_on_err() {
        rt().block_on(async {
            let db = Db::open_in_memory().expect("open");
            db.migrate().await.expect("migrate");
            db.write(|tx| Ok(tx.execute(r#"INSERT INTO "Verification" ("id","identifier","value","expiresAt") VALUES ('a','i','v',0)"#, [])?))
                .await
                .expect("insert");
            let err = db
                .write(|tx| {
                    tx.execute(r#"INSERT INTO "Verification" ("id","identifier","value","expiresAt") VALUES ('b','i','v',0)"#, [])?;
                    Err::<(), _>(DbError::other("abort"))
                })
                .await
                .expect_err("abort");
            assert_eq!(err.message(), "abort");
            let ids = db
                .read(|conn| Ok(conn.query_row(r#"SELECT group_concat("id") FROM "Verification""#, [], |r| r.get::<_, String>(0))?))
                .await
                .expect("read");
            assert_eq!(ids, "a");
        });
    }

    #[test]
    fn readers_run_in_parallel_with_a_write() {
        rt().block_on(async {
            let dir = TempDir::new();
            let db = Arc::new(Db::open(dir.0.join("p.db")).expect("open"));
            db.migrate().await.expect("migrate");
            let reads = (0..12).map(|_| {
                let db = Arc::clone(&db);
                async move {
                    db.read(|conn| {
                        Ok(conn.query_row(r#"SELECT count(*) FROM "Tenant""#, [], |r| {
                            r.get::<_, i64>(0)
                        })?)
                    })
                    .await
                }
            });
            for n in futures_join(reads).await {
                assert_eq!(n.expect("read"), 1);
            }
        });
    }

    async fn futures_join<F>(futures: impl Iterator<Item = F>) -> Vec<F::Output>
    where
        F: std::future::Future + Send + 'static,
        F::Output: Send + 'static,
    {
        let handles: Vec<_> = futures.map(tokio::spawn).collect();
        let mut out = Vec::new();
        for h in handles {
            out.push(h.await.expect("join"));
        }
        out
    }

    #[test]
    fn a_panicking_write_job_rolls_back_and_the_writer_survives() {
        rt().block_on(async {
            let db = Db::open_in_memory().expect("open");
            db.migrate().await.expect("migrate");
            let db = Arc::new(db);
            let db2 = Arc::clone(&db);
            let joined = tokio::spawn(async move {
                db2.write(|tx| -> Result<(), DbError> {
                    tx.execute(r#"INSERT INTO "Verification" ("id","identifier","value","expiresAt") VALUES ('p','i','v',0)"#, [])?;
                    panic!("boom")
                })
                .await
            })
            .await;
            assert!(joined.is_err_and(|e| e.is_panic()));
            let n = db
                .write(|tx| Ok(tx.query_row(r#"SELECT count(*) FROM "Verification""#, [], |r| r.get::<_, i64>(0))?))
                .await
                .expect("writer still works");
            assert_eq!(n, 0);
        });
    }
}

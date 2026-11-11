//! `Db::write` / `Db::read` retried while SQLite reports a transient lock
//! (SQLSTATE `55P03`: a table lock of the shared-cache in-memory database
//! used by tests, or `SQLITE_BUSY` after the busy timeout). The closures are
//! `Clone` so a retry can run them again.

use std::time::Duration;

use chatbotx_db::rusqlite::Connection;
use chatbotx_db::{Db, DbError, WriteTx};

/// Longest pause of the retry (1, 2, 4, ... 256 ms: about 0.5 s in all).
const MAX_PAUSE_MS: u64 = 256;

fn is_transient(e: &DbError) -> bool {
    e.sqlstate() == Some("55P03")
}

pub(crate) async fn write<R, F>(db: &Db, f: F) -> Result<R, DbError>
where
    F: FnOnce(&mut WriteTx<'_>) -> Result<R, DbError> + Clone + Send + 'static,
    R: Send + 'static,
{
    let mut pause = 1u64;
    loop {
        match db.write(f.clone()).await {
            Err(e) if is_transient(&e) && pause <= MAX_PAUSE_MS => {
                tokio::time::sleep(Duration::from_millis(pause)).await;
                pause *= 2;
            }
            other => return other,
        }
    }
}

pub(crate) async fn read<R, F>(db: &Db, f: F) -> Result<R, DbError>
where
    F: FnOnce(&Connection) -> Result<R, DbError> + Clone + Send + 'static,
    R: Send + 'static,
{
    let mut pause = 1u64;
    loop {
        match db.read(f.clone()).await {
            Err(e) if is_transient(&e) && pause <= MAX_PAUSE_MS => {
                tokio::time::sleep(Duration::from_millis(pause)).await;
                pause *= 2;
            }
            other => return other,
        }
    }
}

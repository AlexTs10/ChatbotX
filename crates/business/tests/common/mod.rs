//! Shared fixtures of the ported business tests (P3.6-D1): a migrated SQLite
//! file per test and small row inserts.

#![allow(dead_code)]

use std::ops::Deref;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use chatbotx_db::rusqlite::{Connection, params};
use chatbotx_db::{Db, DbError};

/// A migrated database in a temp directory (removed on drop).
pub struct TestDb {
    db: Db,
    dir: PathBuf,
}

impl Deref for TestDb {
    type Target = Db;
    fn deref(&self) -> &Db {
        &self.db
    }
}

impl Drop for TestDb {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

pub async fn fresh_db() -> TestDb {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "chatbotx-business-test-{}-{nanos}-{n}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let db = Db::open(dir.join("test.db")).expect("Db::open");
    db.migrate().await.expect("migrate");
    TestDb { db, dir }
}

/// Runs `f` in one write transaction (test setup).
pub async fn setup<F, R>(db: &Db, f: F) -> R
where
    F: FnOnce(&Connection) -> Result<R, DbError> + Send + 'static,
    R: Send + 'static,
{
    db.write(move |tx| f(tx)).await.expect("test setup write")
}

/// One query that returns one integer (or `None`).
pub async fn scalar_i64(db: &Db, sql: &'static str, id: i64) -> Option<i64> {
    db.read(move |conn| {
        Ok(conn
            .query_row(sql, params![id], |r| r.get::<_, Option<i64>>(0))
            .ok()
            .flatten())
    })
    .await
    .expect("scalar read")
}

/// Inserts a `User` (`id`) and a `Workspace` (`workspace_id`) it owns.
pub fn workspace(tx: &Connection, user_id: i64, workspace_id: i64) -> Result<i64, DbError> {
    tx.execute(
        r#"INSERT INTO "User" ("id", "email") VALUES (?1, ?2)"#,
        params![user_id, format!("user-{user_id}@example.test")],
    )?;
    tx.execute(
        r#"INSERT INTO "Workspace" ("id", "name", "ownerId") VALUES (?1, ?2, ?3)"#,
        params![workspace_id, format!("Workspace {workspace_id}"), user_id],
    )?;
    Ok(workspace_id)
}

/// Inserts a `Contact` of a workspace.
pub fn contact(tx: &Connection, id: i64, workspace_id: i64) -> Result<i64, DbError> {
    tx.execute(
        r#"INSERT INTO "Contact" ("id", "firstName", "workspaceId") VALUES (?1, 'Ada', ?2)"#,
        params![id, workspace_id],
    )?;
    Ok(id)
}

/// The workspace ids the fixtures use: `WS` and a second one, `OTHER_WS`.
pub const WS: i64 = 2001;
pub const OTHER_WS: i64 = 2002;

/// A fresh database with workspaces `WS` and `OTHER_WS`.
pub async fn two_workspaces() -> TestDb {
    let db = fresh_db().await;
    setup(&db, |tx| {
        workspace(tx, 1001, WS)?;
        workspace(tx, 1002, OTHER_WS)?;
        Ok(())
    })
    .await;
    db
}

//! Shared fixtures of the P3.3 whatsapp-call slice (on top of `common/mod.rs` and
//! `common/repo.rs`; `repo5.rs` is locked).
//!
//! Include after `repo_common`:
//! ```ignore
//! #[path = "common/repo6.rs"]
//! mod repo6;
//! ```

#![allow(dead_code)]

use chatbotx_db::repo::models::WhatsappCall;
use chatbotx_db::repo::{self, Model};
use chatbotx_db::rusqlite::{Connection, params};
use chatbotx_db::{Db, DbError};

use crate::common::{Seed, fresh_db, insert_contact_inbox, insert_conversation, seed};

/// Contact inbox of the call fixtures (seeded contact on the seeded inbox).
pub const CALL_CI: i64 = 3101;
/// Conversation of the call fixtures.
pub const CALL_CONV: i64 = 3201;
/// `2026-08-01T00:00:00.000Z`.
pub const T0: i64 = 1_785_542_400_000;

/// A fresh file with the seed graph, contact inbox `CALL_CI` and conversation `CALL_CONV`.
pub async fn call_db() -> (crate::common::TestDb, Seed) {
    let db = fresh_db().await;
    let s = seed(&db).await;
    db.write(move |tx| {
        insert_contact_inbox(tx, s, CALL_CI, "wa-1", None, None)?;
        insert_conversation(tx, s, CALL_CONV, None)?;
        Ok(())
    })
    .await
    .expect("call fixture");
    (db, s)
}

/// Inserts one `User` (`email` = `agent-<id>@example.test`, `name` = `Agent <id>`).
pub fn put_user(tx: &Connection, id: i64) -> Result<usize, DbError> {
    Ok(tx.execute(
        r#"INSERT INTO "User" ("id", "email", "name") VALUES (?1, ?2, ?3)"#,
        params![
            id,
            format!("agent-{id}@example.test"),
            format!("Agent {id}")
        ],
    )?)
}

/// Inserts one `userInitiated` `ringing` call of the seed workspace / inbox on `CALL_CI` /
/// `CALL_CONV` created at `created_at` (`updatedAt` too), then applies `set` (an SQL SET list,
/// empty for none).
pub fn put_call(tx: &Connection, id: i64, created_at: i64, set: &str) -> Result<usize, DbError> {
    tx.execute(
        r#"INSERT INTO "WhatsappCall" ("id", "createdAt", "updatedAt", "direction", "status",
               "workspaceId", "inboxId", "contactInboxId", "conversationId")
           VALUES (?1, ?2, ?2, 'userInitiated', 'ringing', 2001, 3001, ?3, ?4)"#,
        params![id, created_at, CALL_CI, CALL_CONV],
    )?;
    if set.is_empty() {
        return Ok(1);
    }
    Ok(tx.execute(
        &format!(r#"UPDATE "WhatsappCall" SET {set} WHERE "id" = ?1"#),
        params![id],
    )?)
}

/// The `WhatsappCall` row `id`, if any.
pub async fn call_row(db: &Db, id: i64) -> Option<WhatsappCall> {
    repo::read(db, move |c| {
        repo::find_one::<WhatsappCall>(c, "\"id\" = ?1", &[&id])
    })
    .await
    .expect("read call")
}

/// Every `WhatsappCall` row, by id.
pub async fn call_rows(db: &Db) -> Vec<WhatsappCall> {
    repo::read(db, |c| {
        repo::find_many::<WhatsappCall>(c, "ORDER BY \"id\"", &[])
    })
    .await
    .expect("read calls")
}

/// `WhatsappCall::TABLE` (keeps the `Model` import used).
pub fn call_table() -> &'static str {
    WhatsappCall::TABLE
}
